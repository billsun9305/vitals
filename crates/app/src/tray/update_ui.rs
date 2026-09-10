//! The controller's update behaviour: the check schedule, the menu rows,
//! the alerts. The Objective-C selectors themselves live in
//! `controller.rs` — they must be inside `define_class!` — and each is one
//! line calling into here; this file is where the logic is.
//!
//! Everything here runs on the main thread. The worker threads (a check,
//! and later an install) never touch `self`: they leave their result in a
//! `Slot` and post a notification, and the controller hops back onto the
//! main thread before reading it — the `powerChanged:` pattern.

use std::cell::RefCell;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use objc2::rc::Retained;
use objc2::{sel, DefinedClass};
use objc2_app_kit::{
    NSAlert, NSAlertFirstButtonReturn, NSAlertStyle, NSMenu, NSMenuItem, NSWorkspace,
};
use objc2_foundation::{
    MainThreadMarker, NSBundle, NSNotificationCenter, NSRunLoop, NSRunLoopCommonModes, NSString,
    NSTimer, NSURL,
};

use crate::update::checker::{
    self, CheckOutcome, Slot, UpdateState, CHECK_PERIOD_S, CHECK_TOLERANCE_S, FIRST_CHECK_S,
};
use crate::update::release::{Release, Source, Version};

use super::controller::Controller;

/// Posted from the check thread once its outcome is in the slot.
pub(super) const CHECK_DONE: &str = "com.billsun.vitals.updateCheckDone";

/// Whether this process runs from a `.app` bundle. Only then does the
/// feature exist: a `cargo run` binary has no bundle to replace.
pub(super) fn is_bundled() -> bool {
    NSBundle::mainBundle()
        .bundleURL()
        .path()
        .is_some_and(|p| p.to_string().ends_with(".app"))
}

/// The bundle's path, for the installer. Meaningful only when `is_bundled()`.
///
/// Unused within Task 8: it exists for Task 9's installer worker, its
/// first caller. Unlike the other forward-declared `update::*` interfaces
/// (`install::run`, `relaunch::run`, …), which stay live under
/// `-D dead_code` because `update` is a `pub mod` reachable from the crate
/// root, `update_ui` is a private submodule of `tray` (by this same
/// brief's own `tray/mod.rs`), so no visibility modifier on this function
/// makes it part of the crate's public API — only a call site would. See
/// the Task 8 report's Concerns section.
#[allow(dead_code)]
pub(super) fn app_url() -> PathBuf {
    PathBuf::from(NSBundle::mainBundle().bundlePath().to_string())
}

/// Post `name` on the default centre. Called from worker threads: the
/// observer runs on the posting thread and only hops, so this is safe to
/// call from anywhere.
pub(super) fn post(name: &str) {
    // SAFETY: the name is one of this module's own constants; no object.
    unsafe {
        NSNotificationCenter::defaultCenter()
            .postNotificationName_object(&NSString::from_str(name), None)
    }
}

/// An informational alert with one OK button.
pub(super) fn alert(mtm: MainThreadMarker, message: &str, informative: &str) {
    let alert = NSAlert::new(mtm);
    alert.setAlertStyle(NSAlertStyle::Informational);
    alert.setMessageText(&NSString::from_str(message));
    alert.setInformativeText(&NSString::from_str(informative));
    alert.runModal();
}

/// Open a web page in the default browser.
pub(super) fn open_page(url: &str) {
    match NSURL::URLWithString(&NSString::from_str(url)) {
        Some(ns_url) if NSWorkspace::sharedWorkspace().openURL(&ns_url) => {}
        _ => eprintln!("vitals: could not open {url}"),
    }
}

/// Release notes as the alert shows them: at most 1,500 characters, then `…`.
pub fn truncate_notes(notes: &str) -> String {
    const LIMIT: usize = 1_500;
    let trimmed = notes.trim();
    if trimmed.chars().count() <= LIMIT {
        return trimmed.to_string();
    }
    let mut cut: String = trimmed.chars().take(LIMIT).collect();
    cut.push('…');
    cut
}

/// The menu items this feature owns. Created before `init` so they can
/// live in the ivars; targeted at `self` after (`install_update_items`).
pub(super) struct UpdateItems {
    /// `● Update to Vitals x.y.z…`, present only while an update is available.
    pub(super) row: Retained<NSMenuItem>,
    /// The separator under `row`, present with it.
    pub(super) row_separator: Retained<NSMenuItem>,
    /// `Check for Updates…`.
    pub(super) check: Retained<NSMenuItem>,
}

pub(super) struct UpdateIvars {
    /// `None` outside a bundle: no timers, no items, nothing.
    pub(super) source: Option<Source>,
    pub(super) state: RefCell<UpdateState>,
    pub(super) check_slot: Slot<(CheckOutcome, bool)>,
    /// The 30 s one-shot and the daily repeat, kept so they are not
    /// collected. Never invalidated: they live as long as the tray.
    pub(super) timers: RefCell<Vec<Retained<NSTimer>>>,
    pub(super) items: Option<UpdateItems>,
}

impl UpdateIvars {
    pub(super) fn new(mtm: MainThreadMarker, source: Source) -> UpdateIvars {
        let bundled = is_bundled();
        let items = bundled.then(|| {
            let check = NSMenuItem::new(mtm);
            check.setTitle(&NSString::from_str("Check for Updates…"));
            UpdateItems {
                row: NSMenuItem::new(mtm),
                row_separator: NSMenuItem::separatorItem(mtm),
                check,
            }
        });
        UpdateIvars {
            source: bundled.then_some(source),
            state: RefCell::new(UpdateState::default()),
            check_slot: Arc::new(Mutex::new(None)),
            timers: RefCell::new(Vec::new()),
            items,
        }
    }
}

impl Controller {
    /// Append `Check for Updates…` to `menu` and point the items at `self`.
    /// Runs after `init`. No-op outside a bundle.
    pub(super) fn install_update_items(&self, menu: &NSMenu) {
        let Some(items) = self.ivars().update.items.as_ref() else {
            return;
        };
        items.check.setEnabled(true);
        // SAFETY: `self` responds to `checkForUpdates:` and `installUpdate:`,
        // defined in `controller.rs`.
        unsafe {
            items.check.setTarget(Some(self));
            items.check.setAction(Some(sel!(checkForUpdates:)));
            items.row.setTarget(Some(self));
            items.row.setAction(Some(sel!(installUpdate:)));
        }
        menu.addItem(&items.check);
    }

    /// The 30 s one-shot and the daily repeat, in common modes like the
    /// poll timer, so an open menu does not stall them. No-op outside a
    /// bundle.
    pub(super) fn schedule_update_checks(&self) {
        if self.ivars().update.source.is_none() {
            return;
        }
        let mut timers = Vec::new();
        for (interval, repeats, tolerance) in [
            (FIRST_CHECK_S, false, 5.0),
            (CHECK_PERIOD_S, true, CHECK_TOLERANCE_S),
        ] {
            // SAFETY: `self` responds to `updateTimerFired:`; there is no
            // user info; the mode is Foundation's own constant.
            let timer = unsafe {
                NSTimer::timerWithTimeInterval_target_selector_userInfo_repeats(
                    interval,
                    self,
                    sel!(updateTimerFired:),
                    None,
                    repeats,
                )
            };
            timer.setTolerance(tolerance);
            unsafe { NSRunLoop::currentRunLoop().addTimer_forMode(&timer, NSRunLoopCommonModes) };
            timers.push(timer);
        }
        *self.ivars().update.timers.borrow_mut() = timers;
    }

    /// Start a check unless one is already running or an install is.
    /// `manual` decides whether the outcome is reported in an alert.
    pub(super) fn start_check(&self, manual: bool) {
        let Some(source) = self.ivars().update.source.clone() else {
            return;
        };
        {
            let mut state = self.ivars().update.state.borrow_mut();
            if state.checking || state.installing {
                return;
            }
            state.checking = true;
        }
        let slot = Arc::clone(&self.ivars().update.check_slot);
        checker::spawn_check(source, Version::current(), manual, slot, || {
            post(CHECK_DONE)
        });
    }

    /// On the main thread, after `CHECK_DONE`: fold the outcome in and,
    /// for a manual check, say what happened. An automatic failure is one
    /// line on stderr and nothing else.
    pub(super) fn check_finished(&self) {
        let Some((outcome, manual)) = self.ivars().update.check_slot.lock().unwrap().take() else {
            return;
        };
        if let (CheckOutcome::Failed(reason), false) = (&outcome, manual) {
            eprintln!("vitals: update check failed: {reason}");
        }
        self.ivars()
            .update
            .state
            .borrow_mut()
            .apply(outcome.clone(), Instant::now());
        if !manual {
            return;
        }
        let mtm = MainThreadMarker::from(self);
        match outcome {
            CheckOutcome::UpToDate(latest) => alert(
                mtm,
                "You're up to date",
                &format!("Vitals {latest} is the latest."),
            ),
            CheckOutcome::Failed(reason) => alert(mtm, "Couldn't check for updates", &reason),
            CheckOutcome::Available(release) => self.offer_install(&release),
        }
    }

    /// The row's action: offer the update that is available.
    pub(super) fn install_update(&self) {
        let available = self.ivars().update.state.borrow().available.clone();
        if let Some(release) = available {
            self.offer_install(&release);
        }
    }

    /// The release alert. *View Release* opens the release page.
    pub(super) fn offer_install(&self, release: &Release) {
        let mtm = MainThreadMarker::from(self);
        let alert = NSAlert::new(mtm);
        alert.setAlertStyle(NSAlertStyle::Informational);
        alert.setMessageText(&NSString::from_str(&format!(
            "Vitals {} is available",
            release.version
        )));
        alert.setInformativeText(&NSString::from_str(&truncate_notes(&release.notes)));
        alert.addButtonWithTitle(&NSString::from_str("View Release"));
        alert.addButtonWithTitle(&NSString::from_str("Later"));
        if alert.runModal() == NSAlertFirstButtonReturn {
            open_page(&release.page_url);
        }
    }

    /// Bring the rows in line with the state. Called from `menuWillOpen:`,
    /// so nothing is touched while the menu is closed.
    pub(super) fn refresh_update_items(&self) {
        let Some(items) = self.ivars().update.items.as_ref() else {
            return;
        };
        let Some(menu) = self.ivars().status_item.menu(MainThreadMarker::from(self)) else {
            return;
        };
        let state = self.ivars().update.state.borrow();
        let present = menu.indexOfItem(&items.row) >= 0;
        match (&state.available, present) {
            (Some(release), _) => {
                if !present {
                    menu.insertItem_atIndex(&items.row_separator, 0);
                    menu.insertItem_atIndex(&items.row, 0);
                }
                items.row.setTitle(&NSString::from_str(&format!(
                    "● Update to Vitals {}…",
                    release.version
                )));
                items.row.setEnabled(true);
            }
            (None, true) => {
                menu.removeItem(&items.row);
                menu.removeItem(&items.row_separator);
            }
            (None, false) => {}
        }
        let (title, enabled) = if state.checking {
            ("Checking…", false)
        } else {
            ("Check for Updates…", true)
        };
        items.check.setTitle(&NSString::from_str(title));
        items.check.setEnabled(enabled);
    }
}

#[cfg(test)]
mod tests {
    use super::truncate_notes;

    #[test]
    fn notes_are_trimmed_and_cut_at_fifteen_hundred_characters() {
        assert_eq!(truncate_notes("  ### Added\n- x\n"), "### Added\n- x");
        let exact: String = "é".repeat(1_500);
        assert_eq!(truncate_notes(&exact), exact);
        let long: String = "é".repeat(1_501);
        let cut = truncate_notes(&long);
        assert_eq!(cut.chars().count(), 1_501);
        assert!(cut.ends_with('…'));
        assert!(cut.starts_with(&"é".repeat(1_500)));
    }
}
