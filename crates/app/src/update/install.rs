//! Staging, downloading, verifying and unpacking an update, and swapping
//! it into place. Everything here runs on the installer's worker thread;
//! nothing touches AppKit.
//!
//! The order is the design's: stage next to the bundle (same volume, so
//! the final swap is a rename), download `SHA256SUMS` then the tarball,
//! check the hash, unpack, check the version inside, check the signature,
//! spawn the relaunch helper, swap. The first failure stops the run and
//! `Staged`'s `Drop` removes the staging directory, so the installed bundle
//! is untouched by anything but a successful `swap_into`.

use std::collections::BTreeSet;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_foundation::{
    NSFileManager, NSFileManagerItemReplacementOptions, NSMutableDictionary, NSSearchPathDirectory,
    NSSearchPathDomainMask, NSString, NSURL,
};
use sha2::{Digest, Sha256};

use super::codesign;
use super::http;
use super::release::{expected_sum, Release, Source, UpdateError, Version};

/// The tarball's one top-level entry, and the name the bundle keeps.
pub const BUNDLE_NAME: &str = "Vitals.app";

/// A downloaded, verified, unpacked bundle waiting in a staging directory.
///
/// Dropping it without calling `swap_into` deletes the directory.
pub struct Staged {
    dir: PathBuf,
    app: PathBuf,
    signature_checked: bool,
    swapped: bool,
}

impl Staged {
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The unpacked `Vitals.app` inside the staging directory.
    pub fn app(&self) -> &Path {
        &self.app
    }

    /// Whether step 4 verified the signature, or skipped it because the
    /// running copy has no Team ID.
    pub fn signature_checked(&self) -> bool {
        self.signature_checked
    }

    /// Step 6: replace the bundle at `app_url` with the staged one, then
    /// remove the staging directory. The running process keeps its mapped
    /// binary; only the path changes owner.
    pub fn swap_into(mut self, app_url: &Path) -> Result<(), UpdateError> {
        let original = file_url(app_url);
        let replacement = file_url(&self.app);
        NSFileManager::defaultManager()
            .replaceItemAtURL_withItemAtURL_backupItemName_options_resultingItemURL_error(
                &original,
                &replacement,
                None,
                // `UsingNewMetadataOnly`, not `empty()` (I5): Apple documents
                // the default (`0`) as preserving/merging the *original*
                // item's metadata. An `/Applications/Vitals.app` installed
                // from the DMG carries `com.apple.quarantine`; merging that
                // onto the freshly-downloaded, hash- and signature-verified
                // bundle would defeat the no-quarantine promise (spec §4)
                // for a property nothing in the tarball itself carries.
                NSFileManagerItemReplacementOptions::UsingNewMetadataOnly,
                None,
            )
            .map_err(|e| {
                UpdateError::Io(format!(
                    "replacing {}: {}",
                    app_url.display(),
                    e.localizedDescription()
                ))
            })?;
        self.swapped = true;
        let _ = std::fs::remove_dir_all(&self.dir);
        Ok(())
    }
}

impl Drop for Staged {
    fn drop(&mut self) {
        if !self.swapped {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }
}

/// Steps 1–4: stage next to `app_url`, download, verify, unpack, check.
pub fn prepare(source: &Source, release: &Release, app_url: &Path) -> Result<Staged, UpdateError> {
    prepare_in(staging_dir(app_url)?, source, release)
}

/// `prepare` with a caller-chosen staging directory, which must exist and
/// be empty. It is removed on failure. Tests use this to know where the
/// staging directory was after a failure that returned no `Staged`.
pub fn prepare_in(dir: PathBuf, source: &Source, release: &Release) -> Result<Staged, UpdateError> {
    let mut staged = Staged {
        app: dir.join(BUNDLE_NAME),
        dir,
        signature_checked: false,
        swapped: false,
    };

    // Checked first, but after `staged` exists: a failing check returns
    // through `Staged`'s `Drop`, which removes the staging directory.
    if !release.archive_url.starts_with(&source.download_base) {
        return Err(UpdateError::BadRelease(format!(
            "{} is not under {}",
            release.archive_url, source.download_base
        )));
    }

    // Step 2: download. The sums first, so a bad tarball is never kept.
    let sums_path = http::download(&release.sums_url, &staged.dir)?;
    let archive_path = http::download(&release.archive_url, &staged.dir)?;

    // Step 3: verify.
    let sums = std::fs::read_to_string(&sums_path)?;
    let expected = expected_sum(&sums, &release.archive_name).ok_or_else(|| {
        UpdateError::BadRelease(format!(
            "SHA256SUMS has no entry for {}",
            release.archive_name
        ))
    })?;
    if sha256_of(&archive_path)? != expected {
        return Err(UpdateError::BadChecksum);
    }

    // Step 4: unpack, then the version inside, then the signature.
    unpack(&archive_path, &staged.dir)?;
    let found = bundle_version(&staged.app)?;
    if found != release.version {
        return Err(UpdateError::BadRelease(format!(
            "the archive contains Vitals {found}, not {}",
            release.version
        )));
    }
    match codesign::team_id_of_self() {
        Some(team) => {
            codesign::verify_team(&staged.app, &team).map_err(UpdateError::BadSignature)?;
            staged.signature_checked = true;
        }
        None => eprintln!(
            "vitals: the running copy has no Team ID (ad-hoc signature); skipping the signature check on Vitals {}",
            release.version
        ),
    }
    Ok(staged)
}

/// The whole install: prepare, spawn the helper, swap. The helper is
/// cancelled if the swap fails, so nothing relaunches an unchanged bundle.
/// The caller terminates the app afterwards, on the main thread.
pub fn run(source: &Source, release: &Release, app_url: &Path) -> Result<(), UpdateError> {
    let staged = prepare(source, release, app_url)?;
    let helper = Helper::spawn(std::process::id(), app_url)?;
    match staged.swap_into(app_url) {
        Ok(()) => Ok(()),
        Err(e) => {
            helper.cancel();
            Err(e)
        }
    }
}

/// Step 5: the `vitals relaunch` helper, waiting for this process to exit.
pub struct Helper(Child);

impl Helper {
    pub fn spawn(parent: u32, app_url: &Path) -> Result<Helper, UpdateError> {
        let exe = std::env::current_exe()?;
        let child = Command::new(exe)
            .args(["relaunch", "--parent", &parent.to_string(), "--app"])
            .arg(app_url)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| UpdateError::Io(format!("spawning the relaunch helper: {e}")))?;
        Ok(Helper(child))
    }

    /// SIGTERM the helper (it is blocked in `kevent`, so this is the only
    /// way to stop it) and reap it.
    pub fn cancel(mut self) {
        // SAFETY: `id()` is the pid of a child this process spawned and has
        // not yet reaped, so it cannot have been reused.
        unsafe { libc::kill(self.0.id() as libc::pid_t, libc::SIGTERM) };
        let _ = self.0.wait();
    }
}

/// Step 1: a fresh directory on the same volume as `app_url`, created by
/// Foundation for exactly this purpose (`NSItemReplacementDirectory`).
fn staging_dir(app_url: &Path) -> Result<PathBuf, UpdateError> {
    let near = file_url(app_url);
    let url = NSFileManager::defaultManager()
        .URLForDirectory_inDomain_appropriateForURL_create_error(
            NSSearchPathDirectory::ItemReplacementDirectory,
            NSSearchPathDomainMask::UserDomainMask,
            Some(&near),
            true,
        )
        .map_err(|e| {
            UpdateError::Io(format!(
                "creating a staging directory next to {}: {}",
                app_url.display(),
                e.localizedDescription()
            ))
        })?;
    let path = url
        .path()
        .ok_or_else(|| UpdateError::Io("staging directory has no path".into()))?;
    Ok(PathBuf::from(path.to_string()))
}

fn file_url(path: &Path) -> Retained<NSURL> {
    NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()))
}

fn sha256_of(path: &Path) -> Result<[u8; 32], UpdateError> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher.finalize().into())
}

/// Unpack `archive` into `into`, requiring its top level to be exactly
/// `Vitals.app`. Two passes over the gzip stream: a listing, then the
/// unpack, so nothing is written before the shape is known.
fn unpack(archive: &Path, into: &Path) -> Result<(), UpdateError> {
    let mut top = BTreeSet::new();
    let mut listing = tar::Archive::new(flate2::read::GzDecoder::new(File::open(archive)?));
    for entry in listing.entries()? {
        let entry = entry?;
        let path = entry.path()?;
        let first = path
            .components()
            .find_map(|c| match c {
                std::path::Component::Normal(name) => Some(name.to_string_lossy().into_owned()),
                _ => None,
            })
            .ok_or_else(|| {
                UpdateError::BadRelease(format!("archive entry with no name: {}", path.display()))
            })?;
        top.insert(first);
    }
    if top.len() != 1 || !top.contains(BUNDLE_NAME) {
        return Err(UpdateError::BadRelease(format!(
            "archive top level is {top:?}, expected exactly [{BUNDLE_NAME:?}]"
        )));
    }

    let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(File::open(archive)?));
    // Nothing in the fresh staging directory may be replaced, and the crate
    // rejects entries that would escape `into`. Extended attributes are not
    // unpacked (the default), so no quarantine flag arrives with the bytes;
    // modes are applied masked to 0o777, so executable bits do.
    archive.set_overwrite(false);
    archive.unpack(into)?;
    Ok(())
}

/// `CFBundleShortVersionString` of the bundle at `app`.
///
/// `NSDictionary`'s own `dictionaryWithContentsOfURL` is deprecated in
/// favour of no replacement Apple documents for Rust's purposes; its
/// `NSMutableDictionary` sibling (a subclass, so `objectForKey` below still
/// resolves through `Deref`) is the same read, not deprecated.
fn bundle_version(app: &Path) -> Result<Version, UpdateError> {
    let plist = app.join("Contents/Info.plist");
    let url = file_url(&plist);
    // SAFETY: `url` is a file URL; Foundation returns `None` for anything
    // that is not a property list rather than trusting the bytes.
    let dict: Retained<NSMutableDictionary<NSString, AnyObject>> =
        unsafe { NSMutableDictionary::dictionaryWithContentsOfURL(&url) }.ok_or_else(|| {
            UpdateError::BadRelease(format!(
                "{} is missing or not a property list",
                plist.display()
            ))
        })?;
    let value = dict
        .objectForKey(&NSString::from_str("CFBundleShortVersionString"))
        .ok_or_else(|| {
            UpdateError::BadRelease("Info.plist has no CFBundleShortVersionString".into())
        })?;
    let text = value
        .downcast_ref::<NSString>()
        .ok_or_else(|| {
            UpdateError::BadRelease("CFBundleShortVersionString is not a string".into())
        })?
        .to_string();
    Version::parse(&text)
        .map_err(|e| UpdateError::BadRelease(format!("CFBundleShortVersionString: {e}")))
}
