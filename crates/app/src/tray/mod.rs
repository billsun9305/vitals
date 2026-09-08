//! The menu bar face of `vitals`.
//!
//! `status_item` is pure and unit-tested; `controller` is the AppKit object
//! that renders what it produces. Nothing in this module may be touched off
//! the main thread. `MainThreadMarker` enforces that for callers holding a
//! `&Controller`, but not for the Objective-C runtime, which will happily
//! invoke a registered selector from whatever thread posted the notification;
//! see `controller`'s note on `powerChanged:`.

mod controller;
mod panel;
pub mod status_item;

use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy};
use objc2_foundation::MainThreadMarker;

/// Run the menu bar app. Never returns.
pub fn run() -> ! {
    let mtm = MainThreadMarker::new().expect("tray must run on the main thread");
    let app = NSApplication::sharedApplication(mtm);
    // Accessory: menu bar only, no Dock icon, no menu bar menus of its own.
    app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
    // Held for the lifetime of the run loop: the status item, the menu
    // delegate and the notification centre all reference it unretained.
    let _controller = controller::Controller::new(mtm);
    app.run();
    unreachable!("NSApplication::run does not return")
}
