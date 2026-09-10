//! The dashboard's window: one `NSWindow` and the `WKWebView` that fills it,
//! both alive for exactly as long as this process is.
//!
//! There is no teardown design in this file any more. The old in-process
//! window (see git history) had to drop its `WKWebView` on close and rebuild
//! it on reopen, because it lived inside the long-running tray and a
//! blanked-but-alive view still kept WebKit's helper processes resident for
//! the rest of the tray's life. That problem doesn't exist here: this
//! process hosts nothing else, `mod.rs`'s
//! `applicationShouldTerminateAfterLastWindowClosed:` means the window
//! closing *is* the process exiting, and an exited process takes its
//! `WKWebView` and every WebKit helper behind it down with it for free. So
//! `web_view` is a plain `Retained<WKWebView>` built once in `new` and never
//! detached.
//!
//! Everything in this file runs on the main thread only, via the same
//! `MainThreadMarker` discipline as the rest of the app — see `mod.rs`'s
//! module doc.

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

/// The dashboard's `NSWindow`, plus the `WKWebView` that fills it for the
/// whole life of this process — see the module doc for why there is no
/// detach/reload cycle here any more.
pub struct DashboardWindow {
    window: Retained<NSWindow>,
    web_view: Retained<WKWebView>,
}

impl DashboardWindow {
    /// Build the window and its web view, and load `url` into it.
    ///
    /// Does not show the window — `window::run` fronts it and activates the
    /// app as a separate, explicit step once this returns.
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

    /// Navigate the window's web view to `url`.
    pub fn load(&self, url: &str) {
        let Some(ns_url) = NSURL::URLWithString(&NSString::from_str(url)) else {
            return; // `url` is always ours (an http://127.0.0.1:PORT/); this is belt and braces.
        };
        // SAFETY: `loadRequest:` takes no ownership of its argument and has
        // no requirement beyond AppKit's own main-thread rule, already
        // upheld by every caller holding a `MainThreadMarker`.
        unsafe {
            self.web_view
                .loadRequest(&NSURLRequest::requestWithURL(&ns_url))
        };
    }

    /// Bring the window to the front and make it key, without touching its
    /// content.
    ///
    /// `makeKeyAndOrderFront:` alone does not undo minimization, so a
    /// minimised window would otherwise sit in the Dock while the caller
    /// thinks it's been restored — `isMiniaturized`/`deminiaturize:` first
    /// is what actually brings it back.
    pub fn front(&self) {
        if self.window.isMiniaturized() {
            self.window.deminiaturize(None);
        }
        self.window.makeKeyAndOrderFront(None);
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
