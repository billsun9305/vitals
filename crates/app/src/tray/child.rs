//! Launching and re-fronting the dashboard window's own process.
//!
//! The tray never hosts the dashboard itself any more (see `window`'s
//! module doc for why); all it does is make sure a `vitals window` process
//! is running and bring it forward. `DashboardProcess` remembers that
//! process's pid across calls, so a second "Open Dashboard" click re-fronts
//! the existing window instead of spawning a duplicate — but only the pid,
//! not a `Retained<NSRunningApplication>`, because the pid is what the
//! asynchronous launch completion handler in `launch` below can safely hand
//! back across the thread it runs on. See that function's doc for why.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use objc2::rc::Retained;
use objc2_app_kit::{
    NSApplicationActivationOptions, NSRunningApplication, NSWorkspace, NSWorkspaceOpenConfiguration,
};
use objc2_foundation::{MainThreadMarker, NSArray, NSError, NSString, NSURL};

/// Remembers the pid of the dashboard window process this tray has
/// launched, if any.
pub struct DashboardProcess {
    pid: Arc<Mutex<Option<libc::pid_t>>>,
}

impl DashboardProcess {
    pub fn new() -> Self {
        Self {
            pid: Arc::new(Mutex::new(None)),
        }
    }

    /// Bring the dashboard window to the front, launching it first if it is
    /// not already running.
    ///
    /// `mtm` is the caller's proof this runs on the main thread, matching
    /// every other AppKit-touching function in `tray` — nothing here reads
    /// it directly, since none of `NSWorkspace`/`NSRunningApplication`'s
    /// methods used below are themselves main-thread-gated.
    pub fn show(&self, _mtm: MainThreadMarker, url: &str) -> Result<(), String> {
        if let Some(pid) = *self.pid.lock().unwrap() {
            if let Some(app) = NSRunningApplication::runningApplicationWithProcessIdentifier(pid) {
                if !app.isTerminated() {
                    // Its own `applicationDidBecomeActive:` does the rest —
                    // deminiaturizing and ordering the window front.
                    app.activateWithOptions(NSApplicationActivationOptions::ActivateAllWindows);
                    return Ok(());
                }
            }
        }
        self.launch(url)
    }

    /// Launch a fresh `vitals window` process through LaunchServices.
    ///
    /// `NSWorkspace::openApplicationAtURL:configuration:completionHandler:`
    /// rather than `std::process::Command` is what earns the new process a
    /// proper Dock tile, app-switcher entry and activation grant — a
    /// `posix_spawn`ed child gets none of those under macOS 14's
    /// cooperative activation rules.
    /// `setCreatesNewApplicationInstance(true)` is what makes this open a
    /// genuinely new window process rather than LaunchServices coalescing
    /// onto an already-running one by bundle identifier.
    ///
    /// The completion handler runs asynchronously, on a queue of
    /// LaunchServices' choosing rather than necessarily the main thread or
    /// even one of this process's own existing threads — so it must not
    /// hand a `Retained<NSRunningApplication>` back across that boundary
    /// into `self`. It stores only the launched pid (behind a `Mutex`,
    /// which is `Send`) into `self.pid`. A launch *failure* is reported
    /// with `eprintln!` rather than conjuring an `NSAlert` from whatever
    /// arbitrary thread the handler runs on after `show` has already
    /// returned `Ok` — simpler, and self-correcting: `self.pid` stays
    /// empty, so the next `show` call just retries the launch.
    fn launch(&self, url: &str) -> Result<(), String> {
        let current_exe = std::env::current_exe().map_err(|e| format!("locating vitals: {e}"))?;
        let target = launch_target(&current_exe);
        let exe_url = NSURL::fileURLWithPath(&NSString::from_str(&target.to_string_lossy()));

        let config = NSWorkspaceOpenConfiguration::configuration();
        config.setCreatesNewApplicationInstance(true);
        config.setActivates(true);
        let args: Vec<Retained<NSString>> = [
            "window".to_string(),
            "--url".to_string(),
            url.to_string(),
            "--parent".to_string(),
            std::process::id().to_string(),
        ]
        .iter()
        .map(|s| NSString::from_str(s))
        .collect();
        config.setArguments(&NSArray::from_retained_slice(&args));

        let pid_slot = Arc::clone(&self.pid);
        let handler =
            block2::RcBlock::new(move |app: *mut NSRunningApplication, error: *mut NSError| {
                if let Some(app) = std::ptr::NonNull::new(app) {
                    // SAFETY: a non-null pointer handed to this completion
                    // handler by AppKit is a valid `NSRunningApplication`
                    // for the duration of this call.
                    let app = unsafe { app.as_ref() };
                    *pid_slot.lock().unwrap() = Some(app.processIdentifier());
                } else if let Some(error) = std::ptr::NonNull::new(error) {
                    // SAFETY: same guarantee as above, for the error arm —
                    // AppKit hands back exactly one of the two.
                    let error = unsafe { error.as_ref() };
                    eprintln!(
                        "vitals: failed to open the dashboard window: {}",
                        *error.localizedDescription()
                    );
                }
            });

        NSWorkspace::sharedWorkspace().openApplicationAtURL_configuration_completionHandler(
            &exe_url,
            &config,
            Some(&handler),
        );
        Ok(())
    }
}

impl Default for DashboardProcess {
    fn default() -> Self {
        Self::new()
    }
}

/// The path LaunchServices should be told to open: the surrounding `.app`
/// bundle when `exe` sits inside one at the standard
/// `<name>.app/Contents/MacOS/<exe>` depth, or `exe` itself otherwise — a
/// bare `cargo build` binary has no bundle around it at all.
///
/// Pulled out of `launch` so the bundle-detection logic can be checked
/// without booting AppKit.
fn launch_target(exe: &Path) -> PathBuf {
    let bundle = exe.parent().and_then(Path::parent).and_then(Path::parent);
    match bundle {
        Some(app) if app.extension().is_some_and(|ext| ext == "app") => app.to_path_buf(),
        _ => exe.to_path_buf(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bundled_executable_launches_the_surrounding_app() {
        let exe = Path::new("/Applications/Vitals.app/Contents/MacOS/vitals");
        assert_eq!(launch_target(exe), Path::new("/Applications/Vitals.app"));
    }

    #[test]
    fn a_relative_bundled_executable_also_resolves_to_the_app() {
        let exe = Path::new("./dist/Vitals.app/Contents/MacOS/vitals");
        assert_eq!(launch_target(exe), Path::new("./dist/Vitals.app"));
    }

    #[test]
    fn a_bare_executable_launches_itself() {
        let exe = Path::new("/Users/bill/code/vitals/target/debug/vitals");
        assert_eq!(launch_target(exe), exe);
    }
}
