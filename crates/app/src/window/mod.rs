//! The dashboard window's process: a normal, Dock-visible app whose entire
//! job is "one window, showing one URL, for as long as it is open."
//!
//! Spawned by the tray (`tray::child::DashboardProcess::show`) with `--url`
//! (the address of the server the tray already owns) and `--parent` (the
//! tray's own pid) — see `cli.rs`'s hidden `window` subcommand. Unlike the
//! tray, this process's activation policy is `Regular` from the moment it
//! starts and never changes: it always wants a Dock tile and an
//! app-switcher entry, because showing exactly that is its whole purpose.
//! It exits, taking every WebKit helper process behind its `WKWebView` with
//! it, the instant either the window closes
//! (`applicationShouldTerminateAfterLastWindowClosed:`, below) or the tray
//! that spawned it does (`parent::watch`) — there is no state in which this
//! process outlives having something to show.
//!
//! Everything in this file runs on the main thread only, via the same
//! `MainThreadMarker` discipline `tray` uses.

mod frame;
mod parent;

use std::cell::RefCell;

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{define_class, msg_send, sel, DefinedClass, MainThreadOnly};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSApplicationDelegate, NSMenu, NSMenuItem,
    NSWindowDelegate,
};
use objc2_foundation::{MainThreadMarker, NSNotification, NSString};

use frame::DashboardWindow;

/// Host the dashboard in a window, in a process of its own.
///
/// Never returns: `NSApplication::run()` only stops via this process
/// calling `exit`, which happens either from
/// `applicationShouldTerminateAfterLastWindowClosed:` (the window closed)
/// or from `parent::watch` (the tray exited).
pub fn run(url: &str, parent: u32) -> ! {
    let mtm = MainThreadMarker::new().expect("window must run on the main thread");
    let app = NSApplication::sharedApplication(mtm);
    // Regular from birth, for good: unlike the tray, this process exists
    // only to show a window, so it always wants a Dock tile.
    // `Info.plist`'s `LSUIElement` still governs how the *bundle* would
    // launch on its own; setting the policy here overrides it for this
    // process, the same override the in-process design this replaced
    // already relied on.
    app.setActivationPolicy(NSApplicationActivationPolicy::Regular);

    let delegate = WindowApp::new(mtm, parent);
    app.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
    install_main_menu(mtm, &delegate);

    // Zero CPU while waiting, and exactly one wakeup for the whole life of
    // this process — see `parent.rs`.
    parent::watch(parent);

    let window = DashboardWindow::new(mtm, url, ProtocolObject::from_ref(&*delegate));
    window.front();
    *delegate.ivars().window.borrow_mut() = Some(window);
    app.activate();

    app.run();
    unreachable!("NSApplication::run does not return")
}

/// The App/Edit/Window menu bar, moved here from the tray's `Controller`,
/// which built the equivalent menu for the in-process window design this
/// replaced and has no more use for one at all — the tray's own activation
/// policy never leaves `Accessory` now, so it never has a Dock-visible menu
/// bar to populate.
fn install_main_menu(mtm: MainThreadMarker, delegate: &Retained<WindowApp>) {
    let main_menu = NSMenu::new(mtm);

    // App menu: only Quit. Its own title is irrelevant — AppKit always
    // renders the first top-level item's submenu under the running app's
    // name, substituted automatically.
    let app_menu_item = NSMenuItem::new(mtm);
    let app_menu = NSMenu::new(mtm);
    let quit = NSMenuItem::new(mtm);
    quit.setTitle(&NSString::from_str("Quit Vitals"));
    quit.setKeyEquivalent(&NSString::from_str("q"));
    // SAFETY: `delegate` responds to `quitVitals:`, defined below.
    unsafe {
        quit.setTarget(Some(delegate));
        quit.setAction(Some(sel!(quitVitals:)));
    }
    app_menu.addItem(&quit);
    app_menu_item.setSubmenu(Some(&app_menu));
    main_menu.addItem(&app_menu_item);

    // Edit menu: target `None` on every item, so these route through the
    // responder chain to whatever is first responder — the `WKWebView`,
    // whenever the window is key. Without this, ordinary text editing
    // inside the dashboard (copying a metric, say) would be dead.
    let edit_menu_item = NSMenuItem::new(mtm);
    let edit_menu = NSMenu::new(mtm);
    edit_menu.setTitle(&NSString::from_str("Edit"));
    for (title, key, action) in [
        ("Cut", "x", sel!(cut:)),
        ("Copy", "c", sel!(copy:)),
        ("Paste", "v", sel!(paste:)),
        ("Select All", "a", sel!(selectAll:)),
    ] {
        let item = NSMenuItem::new(mtm);
        item.setTitle(&NSString::from_str(title));
        item.setKeyEquivalent(&NSString::from_str(key));
        // SAFETY: no target is set, so this only registers the selector as
        // the item's action; each one is a standard AppKit editing action
        // that any responder may or may not implement, and a responder
        // that doesn't simply isn't sent it.
        unsafe { item.setAction(Some(action)) };
        edit_menu.addItem(&item);
    }
    edit_menu_item.setSubmenu(Some(&edit_menu));
    main_menu.addItem(&edit_menu_item);

    // Window menu: also target `None`, and additionally registered with
    // `setWindowsMenu` so AppKit manages the standard window list under it.
    let window_menu_item = NSMenuItem::new(mtm);
    let window_menu = NSMenu::new(mtm);
    window_menu.setTitle(&NSString::from_str("Window"));
    let close_item = NSMenuItem::new(mtm);
    close_item.setTitle(&NSString::from_str("Close"));
    close_item.setKeyEquivalent(&NSString::from_str("w"));
    // SAFETY: see the Edit menu above — `performClose:` is one of
    // NSWindow's own standard actions.
    unsafe { close_item.setAction(Some(sel!(performClose:))) };
    window_menu.addItem(&close_item);
    let minimize_item = NSMenuItem::new(mtm);
    minimize_item.setTitle(&NSString::from_str("Minimize"));
    minimize_item.setKeyEquivalent(&NSString::from_str("m"));
    // SAFETY: see above — `performMiniaturize:` is likewise one of
    // NSWindow's own standard actions.
    unsafe { minimize_item.setAction(Some(sel!(performMiniaturize:))) };
    window_menu.addItem(&minimize_item);
    window_menu_item.setSubmenu(Some(&window_menu));
    main_menu.addItem(&window_menu_item);

    let app = NSApplication::sharedApplication(mtm);
    app.setMainMenu(Some(&main_menu));
    app.setWindowsMenu(Some(&window_menu));
}

pub struct Ivars {
    /// The one window this process exists to show. `None` only for the
    /// brief span between `WindowApp::new` and `run` filling it in —
    /// every delegate method below only fires once AppKit's run loop is
    /// live, by which point it is always `Some`.
    window: RefCell<Option<DashboardWindow>>,
    /// The tray's pid, recorded so `quitVitals:` knows who else to signal.
    parent: u32,
}

define_class!(
    // SAFETY:
    // - NSObject has no subclassing requirements.
    // - `WindowApp` does not implement `Drop`.
    #[unsafe(super(NSObject))]
    // Every ivar reaches AppKit; this object must never leave the main
    // thread.
    #[thread_kind = MainThreadOnly]
    #[name = "VitalsWindowApp"]
    #[ivars = Ivars]
    pub struct WindowApp;

    impl WindowApp {
        /// The App-menu "Quit Vitals" action (⌘Q). Quitting the *whole* app
        /// from the window is the standard meaning of ⌘Q in a Dock-visible
        /// app, and this process is only half of it — the tray owns the
        /// server and the menu bar item, so it has to go too, or a "quit"
        /// would leave the dashboard's server (and the tray icon) running
        /// with no window left to show for it.
        #[unsafe(method(quitVitals:))]
        fn quit_vitals(&self, _sender: *mut AnyObject) {
            if let Some(target) = quit_target(self.ivars().parent) {
                // SAFETY: `kill` with a valid pid and `SIGTERM` only
                // requests termination and has no memory-safety
                // requirements of its own; `quit_target` already excluded
                // the pids (0 and 1) that are never this tray's to signal.
                unsafe { libc::kill(target, libc::SIGTERM) };
            }
            NSApplication::sharedApplication(MainThreadMarker::from(self)).terminate(None);
        }
    }

    unsafe impl NSObjectProtocol for WindowApp {}

    unsafe impl NSApplicationDelegate for WindowApp {
        /// The whole lifetime rule for this process: the red button or ⌘W
        /// closes the only window there is, so there is nothing left to
        /// run for. No `windowWillClose:` teardown is needed any more —
        /// exiting the process takes the `WKWebView` and every WebKit
        /// helper behind it down for free.
        #[unsafe(method(applicationShouldTerminateAfterLastWindowClosed:))]
        fn should_terminate_after_last_window_closed(&self, _app: &NSApplication) -> bool {
            true
        }

        /// A Finder double-click (or Dock-icon click) while this process is
        /// already running.
        #[unsafe(method(applicationShouldHandleReopen:hasVisibleWindows:))]
        fn should_handle_reopen(&self, _app: &NSApplication, _has_windows: bool) -> bool {
            self.front();
            false
        }

        /// The other half of `DashboardProcess::show`'s re-front path: the
        /// tray re-activates this process via `NSRunningApplication`, and
        /// this notification is what that activation actually delivers —
        /// it, not the reopen handler above, is what restores a minimised
        /// window on a second "Open Dashboard" click.
        #[unsafe(method(applicationDidBecomeActive:))]
        fn did_become_active(&self, _notification: &NSNotification) {
            self.front();
        }
    }

    unsafe impl NSWindowDelegate for WindowApp {}
);

impl WindowApp {
    fn new(mtm: MainThreadMarker, parent: u32) -> Retained<Self> {
        let ivars = Ivars {
            window: RefCell::new(None),
            parent,
        };
        let this = Self::alloc(mtm).set_ivars(ivars);
        // SAFETY: `init` on a freshly allocated `NSObject` subclass whose
        // ivars were just set is the designated initializer with no
        // further requirements.
        let this: Retained<Self> = unsafe { msg_send![super(this), init] };
        this
    }

    fn front(&self) {
        if let Some(window) = self.ivars().window.borrow().as_ref() {
            window.front();
        }
    }
}

/// Pure decision behind `quitVitals:`: which pid, if any, is worth sending
/// `SIGTERM`.
///
/// `0` means "no real parent recorded" (should not happen — `cli.rs`
/// requires `--parent` — but this stays a clamp rather than a panic) and
/// `1` is `launchd`, never something this process should kill.
fn quit_target(parent: u32) -> Option<i32> {
    (parent > 1).then_some(parent as i32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_and_one_are_never_signalled() {
        assert_eq!(quit_target(0), None);
        assert_eq!(quit_target(1), None);
    }

    #[test]
    fn a_real_pid_is_signalled() {
        assert_eq!(quit_target(37_923), Some(37_923));
    }
}
