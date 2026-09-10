//! The body of the hidden `vitals relaunch` verb: wait for the tray that
//! spawned this process to exit, then open the bundle it installed.
//!
//! Spawned by `update::install::Helper` just before the swap. The wait is
//! the same kqueue registration the dashboard window uses to follow the
//! tray (`window::parent`): zero CPU, one wakeup. The launch goes through
//! LaunchServices rather than `exec`, for the same reasons `tray/child.rs`
//! gives — and because the new bundle must start as its own process, not
//! as a child of this one.

use std::path::Path;
use std::ptr::NonNull;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use objc2_app_kit::{NSRunningApplication, NSWorkspace, NSWorkspaceOpenConfiguration};
use objc2_foundation::{NSDate, NSError, NSRunLoop, NSString, NSURL};

/// How long to wait for LaunchServices to answer once the old process is
/// gone. It normally answers within a second.
const LAUNCH_TIMEOUT: Duration = Duration::from_secs(30);

/// Wait for `parent` to exit, then open `app`.
pub fn run(parent: u32, app: &Path) -> Result<(), String> {
    // A pid that is already gone returns at once — see `wait_for_exit`.
    crate::window::parent::wait_for_exit(parent);
    launch(app)
}

/// Whether `path` looks enough like an app bundle to hand to LaunchServices.
///
/// LaunchServices shows a modal Finder dialog ("The application can't be
/// opened.") when asked to open a path that is not an application, and
/// blocks its completion handler until that dialog is dismissed. This
/// helper is a hidden process with no window and no user watching for a
/// dialog to answer, so it must rule out anything that is not a bundle
/// itself, before LaunchServices ever sees the path. This is not a proof
/// the bundle is well-formed or executable — only enough to catch a plain
/// file or folder.
fn is_app_bundle(path: &Path) -> bool {
    path.is_dir()
        && path.extension().is_some_and(|ext| ext == "app")
        && path.join("Contents").join("MacOS").is_dir()
}

fn launch(app: &Path) -> Result<(), String> {
    if !app.exists() {
        return Err(format!("{} does not exist", app.display()));
    }
    if !is_app_bundle(app) {
        return Err(format!("{} is not an app bundle", app.display()));
    }
    let url = NSURL::fileURLWithPath(&NSString::from_str(&app.to_string_lossy()));
    let config = NSWorkspaceOpenConfiguration::configuration();
    // A new instance, not a re-front of anything already running under the
    // same bundle identifier; and no activation, because a menu bar app
    // has nothing to bring forward.
    config.setCreatesNewApplicationInstance(true);
    config.setActivates(false);

    let (tx, rx) = mpsc::channel::<Result<i32, String>>();
    let handler =
        block2::RcBlock::new(move |app: *mut NSRunningApplication, error: *mut NSError| {
            let result = if let Some(app) = NonNull::new(app) {
                // SAFETY: a non-null pointer handed to this completion
                // handler by AppKit is a valid `NSRunningApplication` for
                // the duration of this call.
                Ok(unsafe { app.as_ref() }.processIdentifier())
            } else if let Some(error) = NonNull::new(error) {
                // SAFETY: same guarantee, for the error arm.
                Err(unsafe { error.as_ref() }.localizedDescription().to_string())
            } else {
                Err("LaunchServices returned neither an application nor an error".to_string())
            };
            let _ = tx.send(result);
        });
    NSWorkspace::sharedWorkspace().openApplicationAtURL_configuration_completionHandler(
        &url,
        &config,
        Some(&handler),
    );

    // The completion handler arrives on a LaunchServices queue, but this
    // process has no `NSApplication` and nothing else drives its main run
    // loop, so drive it here in short slices until the answer lands.
    let deadline = Instant::now() + LAUNCH_TIMEOUT;
    loop {
        if let Ok(result) = rx.try_recv() {
            return result
                .map(|pid| eprintln!("vitals: relaunched {} as pid {pid}", app.display()));
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "LaunchServices did not answer within {}s for {}",
                LAUNCH_TIMEOUT.as_secs(),
                app.display()
            ));
        }
        NSRunLoop::mainRunLoop().runUntilDate(&NSDate::dateWithTimeIntervalSinceNow(0.1));
    }
}

#[cfg(test)]
mod tests {
    use super::is_app_bundle;

    #[test]
    fn a_dot_app_directory_with_contents_macos_is_a_bundle() {
        let dir = tempfile::tempdir().unwrap();
        let app = dir.path().join("Foo.app");
        std::fs::create_dir_all(app.join("Contents").join("MacOS")).unwrap();

        assert!(is_app_bundle(&app));
    }

    #[test]
    fn a_plain_directory_or_an_incomplete_bundle_is_not_a_bundle() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!is_app_bundle(dir.path()));

        let app = dir.path().join("Foo.app");
        std::fs::create_dir_all(&app).unwrap(); // no Contents/MacOS
        assert!(!is_app_bundle(&app));
    }
}
