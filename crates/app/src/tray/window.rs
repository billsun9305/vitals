//! The dashboard's window: one `NSWindow`, and a `WKWebView` that exists
//! only while the window has something to show.
//!
//! The `NSWindow` is built once by `Controller::show_dashboard` and kept for
//! the rest of the run — `setReleasedWhenClosed(false)` means the red
//! button only hides it, and `setFrameAutosaveName` needs the same object
//! back on every reopen to have anything to restore. The `WKWebView` inside
//! it does not share that lifetime: `about:blank` still keeps a loaded page
//! (and WebKit's helper processes behind it — WebContent, Networking, GPU)
//! resident for as long as the view exists, which is the rest of the tray's
//! life once opened even once. That is real idle cost the tray promises not
//! to carry, so a closed window drops the `WKWebView` outright instead of
//! merely navigating it away.
//!
//! The other half of that contract lives in `controller.rs`'s
//! `windowWillClose:`, which calls `detach()` below and flips the
//! activation policy back to `Accessory`. `load()` is what rebuilds the view
//! on the next open. Everything in this file runs on the main thread only,
//! via the same `MainThreadMarker` discipline as the rest of `tray` — see
//! `controller.rs`'s module doc.

use std::cell::RefCell;

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

/// The dashboard's `NSWindow`, plus the `WKWebView` filling it whenever one
/// is loaded.
///
/// `web_view` is `None` exactly when the window is closed (or has never
/// been opened yet): `detach()` clears it, `load()` rebuilds it. That
/// presence/absence is also what `Controller::show_window` reads to tell
/// "still open" from "was closed" — `NSWindow::isVisible` cannot do that
/// job on its own, because it is equally `false` for a window merely
/// minimised to the Dock, which should be restored, not reloaded.
pub struct DashboardWindow {
    window: Retained<NSWindow>,
    web_view: RefCell<Option<Retained<WKWebView>>>,
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

        let this = Self {
            window,
            web_view: RefCell::new(None),
        };
        this.load(mtm, url);
        this
    }

    /// Ensure a `WKWebView` is installed as the window's content view, then
    /// navigate it to `url`.
    ///
    /// Building the view is conditional on `web_view` being empty, so this
    /// single method serves both the first load (from `new`) and a reopen
    /// after `detach()` tore the previous view down — the window itself
    /// (and whatever frame `setFrameAutosaveName` restored onto it) is
    /// never rebuilt, only its content.
    pub fn load(&self, mtm: MainThreadMarker, url: &str) {
        if self.web_view.borrow().is_none() {
            // Size the new view to the window's *current* content area —
            // not the `INITIAL_*` constants — so a reopen after the user
            // resized the window (or after `setFrameAutosaveName` restored
            // a saved size) doesn't snap back to the first-run size before
            // the autoresizing mask catches up.
            let content_size = self
                .window
                .contentRectForFrameRect(self.window.frame())
                .size;
            // SAFETY: `initWithFrame:` is WKWebView's documented way to get
            // a view with the default `WKWebViewConfiguration`, called on a
            // freshly allocated instance.
            let web_view = unsafe {
                WKWebView::initWithFrame(
                    WKWebView::alloc(mtm),
                    NSRect::new(NSPoint::new(0.0, 0.0), content_size),
                )
            };
            // Fills the window and tracks its resizes; there is no other
            // subview competing for space.
            web_view.setAutoresizingMask(
                NSAutoresizingMaskOptions::ViewWidthSizable
                    | NSAutoresizingMaskOptions::ViewHeightSizable,
            );
            self.window.setContentView(Some(&web_view));
            *self.web_view.borrow_mut() = Some(web_view);
        }

        let Some(ns_url) = NSURL::URLWithString(&NSString::from_str(url)) else {
            return; // `url` is always ours (an http://127.0.0.1:PORT/); this is belt and braces.
        };
        let web_view = self.web_view.borrow();
        // SAFETY: `loadRequest:` takes no ownership of its argument and has
        // no requirement beyond AppKit's own main-thread rule, already
        // upheld by every caller holding a `MainThreadMarker`. `web_view` is
        // `Some` unconditionally at this point — either it already was, or
        // the block above just filled it.
        unsafe {
            web_view
                .as_ref()
                .expect("just ensured above")
                .loadRequest(&NSURLRequest::requestWithURL(&ns_url))
        };
    }

    /// Tear the web view down: drop it and clear the window's content view.
    ///
    /// This, not a navigation to `about:blank`, is what makes a closed
    /// window free — a blanked-but-alive `WKWebView` still keeps WebKit's
    /// helper processes (WebContent, Networking, GPU) and the socket to
    /// the dashboard server resident for as long as the view exists. The
    /// `NSWindow` itself survives (`setReleasedWhenClosed(false)`), keeping
    /// whatever frame `setFrameAutosaveName` remembers; only the page and
    /// the process weight behind it go away. `load()` rebuilds it on the
    /// next open.
    pub fn detach(&self) {
        self.window.setContentView(None);
        *self.web_view.borrow_mut() = None;
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

    /// Whether a web view is currently installed — `false` after the user
    /// has closed the window (see `detach()`). This, not
    /// `NSWindow::isVisible`, is the signal `Controller::show_window` uses
    /// to decide whether a reopen needs to `load()` again: `isVisible` is
    /// also `false` for a window that is merely minimised, which should be
    /// restored with its page intact, not reloaded.
    pub fn has_web_view(&self) -> bool {
        self.web_view.borrow().is_some()
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
