//! The dashboard's window: one `NSWindow` hosting one `WKWebView`.
//!
//! Built once by `Controller::show_dashboard` and then kept for the rest of
//! the run — `setReleasedWhenClosed(false)` means the red button only hides
//! it, so reopening is a `makeKeyAndOrderFront:`, not a rebuild. That is
//! also why there is no `Drop` impl or teardown path here: the object lives
//! exactly as long as `Controller` does, and AppKit tears down the real
//! window when the process exits.
//!
//! The other half of the idle-cost contract lives in `controller.rs`'s
//! `windowWillClose:`, which calls `blank()` below and flips the activation
//! policy back to `Accessory`. Everything in this file runs on the main
//! thread only, via the same `MainThreadMarker` discipline as the rest of
//! `tray` — see `controller.rs`'s module doc.

use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::MainThreadOnly;
use objc2_app_kit::{
    NSAutoresizingMaskOptions, NSBackingStoreType, NSScreen, NSWindow, NSWindowDelegate,
    NSWindowStyleMask,
};
use objc2_foundation::{MainThreadMarker, NSPoint, NSRect, NSSize, NSString, NSURLRequest, NSURL};
use objc2_web_kit::WKWebView;

/// Content size the window opens at on first run — big enough to show the
/// dashboard's charts without the user immediately reaching for the resize
/// handle.
const INITIAL_WIDTH: f64 = 1000.0;
const INITIAL_HEIGHT: f64 = 760.0;

/// Below this the dashboard's own layout starts clipping.
const MIN_WIDTH: f64 = 720.0;
const MIN_HEIGHT: f64 = 520.0;

const TITLE: &str = "Vitals";

/// Key AppKit persists the user's chosen frame under between launches. The
/// centred `INITIAL_*` rect above is only ever the first-run default.
const AUTOSAVE_NAME: &str = "dashboard";

/// Deliberately not a real navigation: a page that never loaded can't leave
/// a socket open, a timer running or a render tree behind, which is the
/// whole point of blanking a closed window rather than merely hiding it.
const BLANK_URL: &str = "about:blank";

/// Where a `content`-sized window sits, centred on a `screen`-sized display
/// whose origin is `screen_origin` — a second monitor need not start at
/// `(0, 0)`, and AppKit's screen coordinates put `(0, 0)` at the bottom-left
/// of the main screen.
///
/// Pulled out of `DashboardWindow::new` so the arithmetic can be checked
/// without booting AppKit — see the tests below.
pub fn centered_origin(
    screen_origin: (f64, f64),
    screen: (f64, f64),
    content: (f64, f64),
) -> (f64, f64) {
    (
        screen_origin.0 + (screen.0 - content.0) / 2.0,
        screen_origin.1 + (screen.1 - content.1) / 2.0,
    )
}

/// The dashboard's `NSWindow` plus the `WKWebView` filling it.
pub struct DashboardWindow {
    window: Retained<NSWindow>,
    web_view: Retained<WKWebView>,
}

impl DashboardWindow {
    /// Build the window and load `url` into it. Does not show the window or
    /// touch the app's activation policy — `Controller::show_dashboard`
    /// does both together, because they only make sense as one step (see
    /// its doc comment).
    pub fn new(
        mtm: MainThreadMarker,
        url: &str,
        delegate: &ProtocolObject<dyn NSWindowDelegate>,
    ) -> Self {
        let content = NSSize::new(INITIAL_WIDTH, INITIAL_HEIGHT);
        // `mainScreen` is only `None` on a headless machine (no display
        // attached at all); an origin-anchored fallback still produces a
        // valid, on-screen rect in that case rather than a panic.
        let screen_frame = NSScreen::mainScreen(mtm)
            .map(|s| s.frame())
            .unwrap_or(NSRect::new(NSPoint::new(0.0, 0.0), content));
        let (x, y) = centered_origin(
            (screen_frame.origin.x, screen_frame.origin.y),
            (screen_frame.size.width, screen_frame.size.height),
            (content.width, content.height),
        );
        let content_rect = NSRect::new(NSPoint::new(x, y), content);

        let style = NSWindowStyleMask::Titled
            | NSWindowStyleMask::Closable
            | NSWindowStyleMask::Miniaturizable
            | NSWindowStyleMask::Resizable;

        // SAFETY: `initWithContentRect:styleMask:backing:defer:` is
        // NSWindow's own designated initializer, called on a freshly
        // allocated instance as its type requires; `Buffered` is the only
        // backing store AppKit still implements outside the legacy retained
        // renderer.
        let window = unsafe {
            NSWindow::initWithContentRect_styleMask_backing_defer(
                NSWindow::alloc(mtm),
                content_rect,
                style,
                NSBackingStoreType::Buffered,
                false,
            )
        };
        window.setTitle(&NSString::from_str(TITLE));
        // SAFETY: we hold the window in `Retained` for the controller's
        // whole lifetime instead (see the struct doc); telling AppKit not
        // to also release it on close is what makes "close" reversible.
        unsafe { window.setReleasedWhenClosed(false) };
        // Restores wherever the user last left it; must run after the
        // centred `content_rect` above, which is only the seed for a
        // machine with no saved frame yet.
        window.setFrameAutosaveName(&NSString::from_str(AUTOSAVE_NAME));
        window.setContentMinSize(NSSize::new(MIN_WIDTH, MIN_HEIGHT));
        window.setDelegate(Some(delegate));

        // SAFETY: `initWithFrame:` is WKWebView's documented way to get a
        // view with the default `WKWebViewConfiguration`, called on a
        // freshly allocated instance.
        let web_view = unsafe {
            WKWebView::initWithFrame(
                WKWebView::alloc(mtm),
                NSRect::new(NSPoint::new(0.0, 0.0), content),
            )
        };
        // Fills the window and tracks its resizes; there is no other
        // subview competing for space.
        web_view.setAutoresizingMask(
            NSAutoresizingMaskOptions::ViewWidthSizable
                | NSAutoresizingMaskOptions::ViewHeightSizable,
        );
        window.setContentView(Some(&web_view));

        let this = Self { window, web_view };
        this.load(url);
        this
    }

    /// Navigate to `url`. Used both for the first load and to bring a
    /// reopened window's content back after `blank()` cleared it.
    pub fn load(&self, url: &str) {
        let Some(ns_url) = NSURL::URLWithString(&NSString::from_str(url)) else {
            return; // `url` is always ours (an http://127.0.0.1:PORT/ or the blank sentinel); this is belt and braces.
        };
        // SAFETY: `loadRequest:` takes no ownership of its argument and has
        // no requirement beyond AppKit's own main-thread rule, already
        // upheld by every caller holding a `MainThreadMarker`.
        unsafe {
            self.web_view
                .loadRequest(&NSURLRequest::requestWithURL(&ns_url))
        };
    }

    /// Navigate away from the dashboard so a closed window costs nothing —
    /// see the module doc and `windowWillClose:` in `controller.rs`.
    pub fn blank(&self) {
        self.load(BLANK_URL);
    }

    /// Bring the window to the front and make it key, without touching its
    /// content.
    pub fn front(&self) {
        self.window.makeKeyAndOrderFront(None);
    }

    /// Whether the window is currently on screen. `false` after the user
    /// has closed it — the signal `show_dashboard` uses to decide whether a
    /// reopen needs to `load()` again.
    pub fn is_visible(&self) -> bool {
        self.window.isVisible()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn centers_a_smaller_window_on_a_screen_at_the_origin() {
        assert_eq!(
            centered_origin((0.0, 0.0), (2000.0, 1200.0), (1000.0, 760.0)),
            (500.0, 220.0)
        );
    }

    #[test]
    fn accounts_for_a_screen_that_does_not_start_at_the_origin() {
        // A second monitor to the right of the main one, in AppKit's global
        // screen space.
        assert_eq!(
            centered_origin((1920.0, 0.0), (1920.0, 1080.0), (1000.0, 760.0)),
            (2380.0, 160.0)
        );
    }

    #[test]
    fn a_window_larger_than_the_screen_gets_a_negative_origin_rather_than_clamping() {
        // AppKit clamps what actually lands on screen; this function's job
        // is only the centering arithmetic, and a negative origin is the
        // mathematically correct answer for "centered" once content exceeds
        // the screen.
        assert_eq!(
            centered_origin((0.0, 0.0), (800.0, 600.0), (1000.0, 760.0)),
            (-100.0, -80.0)
        );
    }
}
