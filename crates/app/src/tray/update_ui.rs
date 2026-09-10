//! The controller's update behaviour: the check schedule, the menu rows,
//! the alerts. The Objective-C selectors themselves live in
//! `controller.rs` — they must be inside `define_class!` — and each is one
//! line calling into here; this file is where the logic is.
//!
//! Everything here runs on the main thread. The worker threads (a check,
//! and later an install) never touch `self`: they leave their result in a
//! `Slot` and post a notification, and the controller hops back onto the
//! main thread before reading it — the `powerChanged:` pattern.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use objc2::rc::Retained;
use objc2::{sel, DefinedClass};
use objc2_app_kit::{
    NSAlert, NSAlertFirstButtonReturn, NSAlertStyle, NSAlertThirdButtonReturn, NSApplication,
    NSColor, NSControlStateValueOff, NSControlStateValueOn, NSFont, NSFontAttributeName,
    NSForegroundColorAttributeName, NSMenu, NSMenuItem, NSWorkspace,
};
use objc2_foundation::{
    MainThreadMarker, NSBundle, NSMutableAttributedString, NSNotificationCenter, NSRange,
    NSRunLoop, NSRunLoopCommonModes, NSString, NSTimer, NSURL,
};

use crate::update::checker::{
    self, CheckOutcome, Slot, UpdateState, CHECK_PERIOD_S, CHECK_TOLERANCE_S, FIRST_CHECK_S,
};
use crate::update::install;
use crate::update::release::{Release, Source, Version};

use super::controller::Controller;
use super::login_item::{self, LoginStatus};

/// Posted from the check thread once its outcome is in the slot.
pub(super) const CHECK_DONE: &str = "com.billsun.vitals.updateCheckDone";

/// Posted from the install thread once its result is in the slot.
pub(super) const INSTALL_DONE: &str = "com.billsun.vitals.updateInstallDone";

/// Whether this process runs from a `.app` bundle. Only then does the
/// feature exist: a `cargo run` binary has no bundle to replace.
pub(super) fn is_bundled() -> bool {
    NSBundle::mainBundle()
        .bundleURL()
        .path()
        .is_some_and(|p| p.to_string().ends_with(".app"))
}

/// The bundle's path, for the installer. Meaningful only when `is_bundled()`.
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

/// `text` in `font` and the label colour, with the accent colour over
/// `accent` (UTF-16 location and length). The label colour is set
/// explicitly because an attributed title otherwise draws in black, which
/// a dark menu bar or menu does not tolerate; the accent colour is added
/// last so it wins over the label colour on its range.
pub(super) fn styled(
    text: &str,
    font: &NSFont,
    accent: (usize, usize),
) -> Retained<NSMutableAttributedString> {
    let attributed = NSMutableAttributedString::from_nsstring(&NSString::from_str(text));
    let whole = NSRange::new(0, text.encode_utf16().count());
    let (location, length) = accent;
    // SAFETY: the keys are AppKit's own constants; the values are an
    // NSFont and NSColors, the types those keys take; both ranges lie
    // within the string, whose length is measured the way NSRange counts.
    unsafe {
        attributed.addAttribute_value_range(NSFontAttributeName, font, whole);
        attributed.addAttribute_value_range(
            NSForegroundColorAttributeName,
            &NSColor::labelColor(),
            whole,
        );
        attributed.addAttribute_value_range(
            NSForegroundColorAttributeName,
            &NSColor::controlAccentColor(),
            NSRange::new(location, length),
        );
    }
    attributed
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
    /// `Start at Login`, refreshed from the registration on every open.
    pub(super) login: Retained<NSMenuItem>,
    /// `Check for Updates…`.
    pub(super) check: Retained<NSMenuItem>,
}

pub(super) struct UpdateIvars {
    /// `None` outside a bundle: no timers, no items, nothing.
    pub(super) source: Option<Source>,
    pub(super) state: RefCell<UpdateState>,
    pub(super) check_slot: Slot<(CheckOutcome, bool)>,
    /// The version that was being installed travels with the result, so
    /// `install_finished` names the release that was actually installed
    /// even if a check that landed in between replaced `state.available`.
    pub(super) install_slot: Slot<(Version, Result<(), String>)>,
    /// The 30 s one-shot and the daily repeat, kept so they are not
    /// collected. Never invalidated: they live as long as the tray.
    pub(super) timers: RefCell<Vec<Retained<NSTimer>>>,
    pub(super) items: Option<UpdateItems>,
    /// A release the user chose to install while a check was in flight
    /// (I1): parked here by `start_install` instead of being dropped, and
    /// installed by `check_finished` once that check lands.
    pub(super) pending_install: RefCell<Option<Release>>,
    /// A manual *Check for Updates…* click that arrived while a check was
    /// already running (ledger #85): promotes that check's landing to
    /// report as manual, so the click still gets its alert.
    pub(super) manual_pending: Cell<bool>,
}

impl UpdateIvars {
    pub(super) fn new(mtm: MainThreadMarker, source: Source) -> UpdateIvars {
        let bundled = is_bundled();
        let items = bundled.then(|| {
            let login = NSMenuItem::new(mtm);
            login.setTitle(&NSString::from_str("Start at Login"));
            let check = NSMenuItem::new(mtm);
            check.setTitle(&NSString::from_str("Check for Updates…"));
            UpdateItems {
                row: NSMenuItem::new(mtm),
                row_separator: NSMenuItem::separatorItem(mtm),
                login,
                check,
            }
        });
        UpdateIvars {
            source: bundled.then_some(source),
            state: RefCell::new(UpdateState::default()),
            check_slot: Arc::new(Mutex::new(None)),
            install_slot: Arc::new(Mutex::new(None)),
            timers: RefCell::new(Vec::new()),
            items,
            pending_install: RefCell::new(None),
            manual_pending: Cell::new(false),
        }
    }
}

/// What `start_install` should do, given the state flags at the moment it
/// is called — pure, so it is unit-tested without AppKit. An install
/// already running blocks a second one; a check in flight defers instead
/// of silently dropping the click (I1), since the check's landing retries
/// it once `checking` clears.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum InstallAction {
    Blocked,
    Defer,
    Start,
}

pub(super) fn install_action(checking: bool, installing: bool) -> InstallAction {
    if installing {
        InstallAction::Blocked
    } else if checking {
        InstallAction::Defer
    } else {
        InstallAction::Start
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
        // SAFETY: `self` responds to `checkForUpdates:`, `installUpdate:` and
        // `toggleLoginItem:`, defined in `controller.rs`.
        unsafe {
            items.check.setTarget(Some(self));
            items.check.setAction(Some(sel!(checkForUpdates:)));
            items.row.setTarget(Some(self));
            items.row.setAction(Some(sel!(installUpdate:)));
            items.login.setTarget(Some(self));
            items.login.setAction(Some(sel!(toggleLoginItem:)));
        }
        menu.addItem(&items.login);
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
    /// `manual` decides whether the outcome is reported in an alert. A
    /// manual click while a check is already running does not start a
    /// second one; it promotes the running check's landing to report as
    /// manual instead (ledger #85), rather than doing nothing.
    pub(super) fn start_check(&self, manual: bool) {
        let Some(source) = self.ivars().update.source.clone() else {
            return;
        };
        {
            let mut state = self.ivars().update.state.borrow_mut();
            if state.checking || state.installing {
                if manual && state.checking {
                    self.ivars().update.manual_pending.set(true);
                }
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
    /// line on stderr and nothing else. A release parked in
    /// `pending_install` by `start_install` (I1) is installed now that
    /// `checking` is clear, ahead of reporting this check's own outcome —
    /// the user already chose that release, and the installer re-verifies
    /// hash and signature regardless of what this check found.
    pub(super) fn check_finished(&self) {
        let Some((outcome, manual)) = self.ivars().update.check_slot.lock().unwrap().take() else {
            return;
        };
        let manual = manual || self.ivars().update.manual_pending.replace(false);
        if let (CheckOutcome::Failed(reason), false) = (&outcome, manual) {
            eprintln!("vitals: update check failed: {reason}");
        }
        self.ivars()
            .update
            .state
            .borrow_mut()
            .apply(outcome.clone(), Instant::now());
        self.refresh_title();
        // Bound to a `let` so the `RefMut` is released before `start_install`
        // runs — it may touch the same cell.
        let pending = self.ivars().update.pending_install.borrow_mut().take();
        if let Some(release) = pending {
            self.start_install(release);
            return;
        }
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

    /// The release alert: *Install and Relaunch* (default), *Later*,
    /// *View Release*. *Later* changes nothing; the dot stays.
    pub(super) fn offer_install(&self, release: &Release) {
        let mtm = MainThreadMarker::from(self);
        let alert = NSAlert::new(mtm);
        alert.setAlertStyle(NSAlertStyle::Informational);
        alert.setMessageText(&NSString::from_str(&format!(
            "Vitals {} is available",
            release.version
        )));
        alert.setInformativeText(&NSString::from_str(&truncate_notes(&release.notes)));
        alert.addButtonWithTitle(&NSString::from_str("Install and Relaunch"));
        alert.addButtonWithTitle(&NSString::from_str("Later"));
        alert.addButtonWithTitle(&NSString::from_str("View Release"));
        let response = alert.runModal();
        if response == NSAlertFirstButtonReturn {
            self.start_install(release.clone());
        } else if response == NSAlertThirdButtonReturn {
            open_page(&release.page_url);
        }
    }

    /// Run the install on a worker thread. Its result comes back through
    /// `INSTALL_DONE`. An install already running refuses a second one. A
    /// check in flight (I1) does not drop the click either: its landing
    /// would overwrite `state.available` out from under a install started
    /// now, so the release is parked in `pending_install` instead and
    /// `check_finished` starts it once that check lands; the row reads
    /// `Installing…` either way.
    pub(super) fn start_install(&self, release: Release) {
        let Some(source) = self.ivars().update.source.clone() else {
            return;
        };
        let action = {
            let mut state = self.ivars().update.state.borrow_mut();
            let action = install_action(state.checking, state.installing);
            if action == InstallAction::Start {
                state.installing = true;
            }
            action
        };
        match action {
            InstallAction::Blocked => return,
            InstallAction::Defer => {
                *self.ivars().update.pending_install.borrow_mut() = Some(release);
                // The row reads *Installing…* and is disabled from here on,
                // including on the next `menuWillOpen:`, so the click is
                // visibly honoured and cannot be re-issued (or "Later"-ed)
                // while it waits for the check to land.
                self.refresh_update_items();
                return;
            }
            InstallAction::Start => {}
        }
        let version = release.version;
        let app_url = app_url();
        let slot = Arc::clone(&self.ivars().update.install_slot);
        std::thread::Builder::new()
            .name("vitals-update-install".into())
            .spawn(move || {
                let result = install::run(&source, &release, &app_url).map_err(|e| e.to_string());
                *slot.lock().unwrap() = Some((version, result));
                post(INSTALL_DONE);
            })
            .expect("spawning the install thread");
    }

    /// On the main thread, after `INSTALL_DONE`. Success means the new
    /// bundle is in place and the helper is waiting for this process to
    /// exit: terminate. Failure means nothing changed: say why, naming the
    /// version that was actually installed — it travelled with the result
    /// rather than being re-read from `state.available`, which a check
    /// landing during the install may since have replaced or cleared.
    pub(super) fn install_finished(&self) {
        let Some((version, result)) = self.ivars().update.install_slot.lock().unwrap().take()
        else {
            return;
        };
        let mtm = MainThreadMarker::from(self);
        self.ivars().update.state.borrow_mut().installing = false;
        match result {
            Ok(()) => NSApplication::sharedApplication(mtm).terminate(None),
            Err(reason) => alert(mtm, &format!("Couldn't install Vitals {version}"), &reason),
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
        // A release parked behind an in-flight check is an install too.
        let installing = state.installing || self.ivars().update.pending_install.borrow().is_some();
        let present = menu.indexOfItem(&items.row) >= 0;
        match (&state.available, present) {
            (Some(release), _) => {
                if !present {
                    menu.insertItem_atIndex(&items.row_separator, 0);
                    menu.insertItem_atIndex(&items.row, 0);
                }
                if installing {
                    items.row.setAttributedTitle(None);
                    items.row.setTitle(&NSString::from_str("Installing…"));
                    items.row.setEnabled(false);
                } else {
                    let title = format!("● Update to Vitals {}…", release.version);
                    items.row.setAttributedTitle(Some(&styled(
                        &title,
                        &NSFont::menuFontOfSize(0.0),
                        (0, 1),
                    )));
                    items.row.setEnabled(true);
                }
            }
            (None, true) => {
                menu.removeItem(&items.row);
                menu.removeItem(&items.row_separator);
            }
            (None, false) => {}
        }
        let title = if state.checking {
            "Checking…"
        } else {
            "Check for Updates…"
        };
        items.check.setTitle(&NSString::from_str(title));
        // Disabled during an install too (ledger #101): a click here while
        // installing would start a second, overlapping check.
        items.check.setEnabled(!state.checking && !installing);
    }

    /// The *Start at Login* row, from the live registration.
    pub(super) fn refresh_login_item(&self) {
        let Some(items) = self.ivars().update.items.as_ref() else {
            return;
        };
        let (title, checked, enabled) = login_item::menu_state(login_item::status());
        items.login.setTitle(&NSString::from_str(title));
        items.login.setState(if checked {
            NSControlStateValueOn
        } else {
            NSControlStateValueOff
        });
        items.login.setEnabled(enabled);
    }

    /// The row's action: flip the registration, or open the Settings
    /// pane when macOS is waiting for approval.
    pub(super) fn toggle_login_item(&self) {
        let result = match login_item::status() {
            LoginStatus::Enabled => login_item::set(false),
            LoginStatus::NotRegistered => login_item::set(true),
            LoginStatus::RequiresApproval => {
                login_item::open_settings();
                Ok(())
            }
            LoginStatus::NotFound => Ok(()),
        };
        if let Err(reason) = result {
            alert(
                MainThreadMarker::from(self),
                "Couldn't change Start at Login",
                &reason,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{install_action, truncate_notes, InstallAction};

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

    /// (I1) A click that reaches `start_install` never gets silently
    /// dropped: it either starts, is deferred to run once the in-flight
    /// check lands, or is blocked because an install is already running.
    #[test]
    fn install_action_defers_to_a_running_check_instead_of_dropping_the_click() {
        assert_eq!(install_action(false, false), InstallAction::Start);
        assert_eq!(install_action(true, false), InstallAction::Defer);
        assert_eq!(install_action(false, true), InstallAction::Blocked);
        // An install already running wins even if `checking` is also set.
        assert_eq!(install_action(true, true), InstallAction::Blocked);
    }
}
