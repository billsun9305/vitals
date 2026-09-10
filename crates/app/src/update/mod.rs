//! In-app updates: finding the latest GitHub Release, downloading and
//! verifying it, swapping it into place and relaunching.
//!
//! The split follows what each file may touch. `release` is pure and owns
//! every decision that can be unit-tested; `http` talks to Foundation's
//! `NSURLSession`; `codesign` to the Security framework; `install` to the
//! file system; `relaunch` to LaunchServices; `checker` runs a check on a
//! worker thread and keeps the state the tray shows. Nothing here touches
//! AppKit — the tray's own `update_ui` does that.

pub mod codesign;
pub mod http;
pub mod install;
pub mod relaunch;
pub mod release;
