//! The installer end to end, against a loopback release server.
//!
//! No process is launched: `Helper::spawn` is never called, and
//! `swap_into` replaces a temporary "installed" bundle under
//! `tempfile::tempdir()`, never a real one. The fixture builds the tarball
//! itself from a bundle whose Info.plist carries the target version.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use sha2::{Digest, Sha256};
use vitals::update::install::{prepare, prepare_in};
use vitals::update::release::{archive_name, parse_latest, Release, Source, UpdateError, Version};

const OLD: &str = "0.1.0";
const NEW: &str = "0.1.1";

/// A minimal bundle: an Info.plist with the version, and an executable.
fn make_bundle(dir: &Path, version: &str) -> PathBuf {
    let app = dir.join("Vitals.app");
    fs::create_dir_all(app.join("Contents/MacOS")).unwrap();
    fs::write(
        app.join("Contents/Info.plist"),
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
             <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
             <plist version=\"1.0\"><dict>\n\
             <key>CFBundleIdentifier</key><string>com.billsun.vitals</string>\n\
             <key>CFBundleShortVersionString</key><string>{version}</string>\n\
             </dict></plist>\n"
        ),
    )
    .unwrap();
    let exe = app.join("Contents/MacOS/vitals");
    fs::write(&exe, format!("#!/bin/sh\necho vitals {version}\n")).unwrap();
    fs::set_permissions(&exe, fs::Permissions::from_mode(0o755)).unwrap();
    app
}

/// `app` as a gzipped tar whose one top-level entry is `Vitals.app/`,
/// plus an optional second top-level file for the "two entries" case.
fn tarball(app: &Path, extra_top_level: Option<&str>) -> Vec<u8> {
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    {
        let mut tar = tar::Builder::new(&mut gz);
        tar.append_dir_all("Vitals.app", app).unwrap();
        if let Some(name) = extra_top_level {
            let mut header = tar::Header::new_gnu();
            header.set_size(0);
            header.set_mode(0o644);
            header.set_cksum();
            tar.append_data(&mut header, name, std::io::empty())
                .unwrap();
        }
        tar.finish().unwrap();
    }
    gz.finish().unwrap()
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn plist_version(app: &Path) -> String {
    let text = fs::read_to_string(app.join("Contents/Info.plist")).unwrap();
    text.split("<key>CFBundleShortVersionString</key><string>")
        .nth(1)
        .unwrap()
        .split("</string>")
        .next()
        .unwrap()
        .to_string()
}

fn has_quarantine(path: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    let path = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
    // SAFETY: both strings are valid and NUL-terminated; a null buffer of
    // size 0 only asks whether the attribute exists.
    let n = unsafe {
        libc::getxattr(
            path.as_ptr(),
            c"com.apple.quarantine".as_ptr(),
            std::ptr::null_mut(),
            0,
            0,
            0,
        )
    };
    n >= 0
}

/// Every file under `dir` with its bytes, sorted, for "unchanged" checks.
fn snapshot(dir: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    fn walk(dir: &Path, out: &mut Vec<(PathBuf, Vec<u8>)>) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(&path, out);
            } else {
                out.push((path.clone(), fs::read(&path).unwrap()));
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, &mut out);
    out.sort();
    out
}

/// What the fixture serves. Statuses are 200 unless a test overrides one.
struct Files {
    latest: Vec<u8>,
    sums: Vec<u8>,
    sums_status: u16,
    archive: Vec<u8>,
    archive_name: String,
    archive_status: u16,
}

impl Files {
    /// A self-consistent release of `version` built from `archive`: the
    /// asset is named for the version and `SHA256SUMS` carries its digest.
    fn consistent(base: &str, version: &str, archive: Vec<u8>) -> Files {
        let name = archive_name(Version::parse(version).unwrap());
        let sums = format!("{}  {name}\n", sha256_hex(&archive));
        Files {
            latest: latest_json(base, version, &name),
            sums: sums.into_bytes(),
            sums_status: 200,
            archive,
            archive_name: name,
            archive_status: 200,
        }
    }
}

fn latest_json(base: &str, version: &str, archive_name: &str) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "tag_name": format!("v{version}"),
        "draft": false,
        "prerelease": false,
        "html_url": format!("{base}releases/tag/v{version}"),
        "body": "Test release.",
        "assets": [
            {"name": archive_name, "browser_download_url": format!("{base}{archive_name}")},
            {"name": "SHA256SUMS", "browser_download_url": format!("{base}SHA256SUMS")}
        ]
    }))
    .unwrap()
}

/// A loopback release server. `start` binds so the base URL is known
/// before the files (which embed it) are built; `serve` starts answering.
struct Fixture {
    base: String,
    server: Arc<tiny_http::Server>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Fixture {
    fn start() -> Fixture {
        let server = Arc::new(tiny_http::Server::http("127.0.0.1:0").unwrap());
        let port = server.server_addr().to_ip().unwrap().port();
        Fixture {
            base: format!("http://127.0.0.1:{port}/"),
            server,
            thread: None,
        }
    }

    fn serve(&mut self, files: Files) {
        let server = Arc::clone(&self.server);
        self.thread = Some(std::thread::spawn(move || {
            for request in server.incoming_requests() {
                let url = request.url().to_string();
                let (status, body) = if url == "/releases/latest" {
                    (200, files.latest.clone())
                } else if url == "/SHA256SUMS" {
                    (files.sums_status, files.sums.clone())
                } else if url == format!("/{}", files.archive_name) {
                    (files.archive_status, files.archive.clone())
                } else {
                    (404, b"not found".to_vec())
                };
                let _ =
                    request.respond(tiny_http::Response::from_data(body).with_status_code(status));
            }
        }));
    }

    fn source(&self) -> Source {
        Source::loopback(&self.base).unwrap()
    }

    fn release(&self) -> Release {
        let source = self.source();
        let json = vitals::update::http::fetch(&source.api_latest).unwrap();
        parse_latest(&json, &source).unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.server.unblock();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// Run `prepare_in` against `files` and require it to fail: the error is
/// returned, and the staging directory and the installed bundle are
/// asserted gone and unchanged respectively.
fn expect_failure(files: Files, fx: &mut Fixture, installed: &Path, tmp: &Path) -> UpdateError {
    fx.serve(files);
    let (source, release) = (fx.source(), fx.release());
    expect_failure_for(&source, &release, installed, tmp)
}

fn expect_failure_for(
    source: &Source,
    release: &Release,
    installed: &Path,
    tmp: &Path,
) -> UpdateError {
    let staging = tmp.join("staging");
    fs::create_dir_all(&staging).unwrap();
    let before = snapshot(installed);
    let err = prepare_in(staging.clone(), source, release)
        .err()
        .expect("prepare must fail");
    assert!(
        !staging.exists(),
        "staging directory left behind after {err}"
    );
    assert_eq!(
        snapshot(installed),
        before,
        "installed bundle changed after {err}"
    );
    assert_eq!(plist_version(installed), OLD);
    err
}

#[test]
fn a_good_release_replaces_the_installed_bundle() {
    let tmp = tempfile::tempdir().unwrap();
    let installed = make_bundle(&tmp.path().join("installed"), OLD);
    let new_app = make_bundle(&tmp.path().join("new"), NEW);
    let mut fx = Fixture::start();
    let base = fx.base.clone();
    fx.serve(Files::consistent(&base, NEW, tarball(&new_app, None)));
    let (source, release) = (fx.source(), fx.release());
    assert_eq!(release.version, Version::parse(NEW).unwrap());

    let staged = prepare(&source, &release, &installed).unwrap();
    assert!(
        !staged.signature_checked(),
        "the test binary has no Team ID, so the signature check must be skipped"
    );
    let staging_dir = staged.dir().to_path_buf();
    assert!(staging_dir.exists());
    assert_eq!(plist_version(staged.app()), NEW);

    staged.swap_into(&installed).unwrap();

    assert!(
        !staging_dir.exists(),
        "staging directory should be gone after the swap"
    );
    assert_eq!(plist_version(&installed), NEW);
    let exe = installed.join("Contents/MacOS/vitals");
    assert_eq!(
        fs::read_to_string(&exe).unwrap(),
        format!("#!/bin/sh\necho vitals {NEW}\n")
    );
    assert_ne!(
        fs::metadata(&exe).unwrap().permissions().mode() & 0o111,
        0,
        "executable bit lost"
    );
    assert!(
        !has_quarantine(&exe),
        "quarantine flag on the new executable"
    );
    assert!(
        !has_quarantine(&installed),
        "quarantine flag on the new bundle"
    );
}

#[test]
fn a_wrong_hash_is_rejected_and_nothing_changes() {
    let tmp = tempfile::tempdir().unwrap();
    let installed = make_bundle(&tmp.path().join("installed"), OLD);
    let new_app = make_bundle(&tmp.path().join("new"), NEW);
    let mut fx = Fixture::start();
    let base = fx.base.clone();
    let mut files = Files::consistent(&base, NEW, tarball(&new_app, None));
    files.sums = format!("{}  {}\n", "0".repeat(64), files.archive_name).into_bytes();
    let err = expect_failure(files, &mut fx, &installed, tmp.path());
    assert_eq!(err, UpdateError::BadChecksum);
}

#[test]
fn a_tarball_carrying_the_wrong_version_is_rejected() {
    let tmp = tempfile::tempdir().unwrap();
    let installed = make_bundle(&tmp.path().join("installed"), OLD);
    // The release says 0.1.1, the bundle inside says 0.1.0; the hash matches.
    let stale_app = make_bundle(&tmp.path().join("stale"), OLD);
    let mut fx = Fixture::start();
    let base = fx.base.clone();
    let files = Files::consistent(&base, NEW, tarball(&stale_app, None));
    let err = expect_failure(files, &mut fx, &installed, tmp.path());
    assert!(matches!(err, UpdateError::BadRelease(_)), "{err}");
}

#[test]
fn a_tarball_with_two_top_level_entries_is_rejected() {
    let tmp = tempfile::tempdir().unwrap();
    let installed = make_bundle(&tmp.path().join("installed"), OLD);
    let new_app = make_bundle(&tmp.path().join("new"), NEW);
    let mut fx = Fixture::start();
    let base = fx.base.clone();
    let files = Files::consistent(&base, NEW, tarball(&new_app, Some("README.txt")));
    let err = expect_failure(files, &mut fx, &installed, tmp.path());
    assert!(matches!(err, UpdateError::BadRelease(_)), "{err}");
}

#[test]
fn server_errors_are_reported_and_nothing_changes() {
    let tmp = tempfile::tempdir().unwrap();
    let installed = make_bundle(&tmp.path().join("installed"), OLD);
    let new_app = make_bundle(&tmp.path().join("new"), NEW);

    // SHA256SUMS is a 404.
    {
        let mut fx = Fixture::start();
        let base = fx.base.clone();
        let mut files = Files::consistent(&base, NEW, tarball(&new_app, None));
        files.sums_status = 404;
        let err = expect_failure(files, &mut fx, &installed, tmp.path());
        assert!(
            matches!(&err, UpdateError::Http(r) if r.contains("404")),
            "{err}"
        );
    }
    // The tarball is a 403.
    {
        let mut fx = Fixture::start();
        let base = fx.base.clone();
        let mut files = Files::consistent(&base, NEW, tarball(&new_app, None));
        files.archive_status = 403;
        let err = expect_failure(files, &mut fx, &installed, tmp.path());
        assert!(
            matches!(&err, UpdateError::Http(r) if r.contains("403")),
            "{err}"
        );
    }
    // Nothing listens at all.
    {
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let base = format!("http://127.0.0.1:{port}/");
        let source = Source::loopback(&base).unwrap();
        let name = archive_name(Version::parse(NEW).unwrap());
        let release = Release {
            version: Version::parse(NEW).unwrap(),
            notes: String::new(),
            page_url: base.clone(),
            archive_url: format!("{base}{name}"),
            sums_url: format!("{base}SHA256SUMS"),
            archive_name: name,
        };
        let err = expect_failure_for(&source, &release, &installed, tmp.path());
        assert!(matches!(err, UpdateError::Http(_)), "{err}");
    }
}
