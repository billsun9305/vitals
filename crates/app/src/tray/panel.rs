//! Dropdown geometry and drawing.
//!
//! All arithmetic lives in the pure functions below so it can be tested; the
//! `drawRect:` body is only CoreGraphics calls against their output.

use objc2::rc::Retained;
use objc2::{define_class, msg_send, DefinedClass, MainThreadOnly};
use objc2_app_kit::{NSBezierPath, NSColor, NSView};
use objc2_foundation::{MainThreadMarker, NSPoint, NSRect, NSSize};
use std::cell::RefCell;
use vitals_core::ring::Ring;

pub const PANEL_WIDTH: f64 = 260.0;
pub const PANEL_HEIGHT: f64 = 96.0;
const BAR_GAP: f64 = 3.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// One evenly spaced full-height column per core, tiling `width`.
pub fn bar_rects(width: f64, height: f64, cores: usize) -> Vec<Rect> {
    if cores == 0 {
        return Vec::new();
    }
    let total_gap = BAR_GAP * (cores.saturating_sub(1)) as f64;
    let bar_w = ((width - total_gap) / cores as f64).max(1.0);
    (0..cores)
        .map(|i| Rect {
            x: i as f64 * (bar_w + BAR_GAP),
            y: 0.0,
            w: bar_w,
            h: height,
        })
        .collect()
}

/// Map a series of 0–100 percentages onto a `width` x `height` box, oldest
/// on the left. A single sample sits at x=0 at its own height.
pub fn sparkline_points(values: &[f32], width: f64, height: f64) -> Vec<(f64, f64)> {
    let n = values.len();
    if n == 0 {
        return Vec::new();
    }
    let step = if n > 1 { width / (n - 1) as f64 } else { 0.0 };
    values
        .iter()
        .enumerate()
        .map(|(i, v)| {
            let y = (*v as f64 / 100.0).clamp(0.0, 1.0) * height;
            (i as f64 * step, (y * 10.0).round() / 10.0)
        })
        .collect()
}

/// State backing the custom-drawn dropdown view. Written only while the
/// menu is open (see `Controller::apply_sample` and `menuDidClose:`), so an
/// idle tray never touches it.
pub struct PanelState {
    pub core_pcts: Vec<f32>,
    pub cpu_history: Ring<f32>,
}

define_class!(
    // SAFETY:
    // - NSView has no subclassing requirements beyond overriding drawRect:.
    // - `PanelView` does not implement `Drop`.
    #[unsafe(super(NSView))]
    // Hosted inside an `NSMenuItem`'s view while the menu is open; AppKit
    // only ever calls `drawRect:` on the main thread.
    #[thread_kind = MainThreadOnly]
    #[name = "VitalsPanelView"]
    #[ivars = RefCell<PanelState>]
    pub struct PanelView;

    impl PanelView {
        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, _dirty: NSRect) {
            let state = self.ivars().borrow();
            let bounds = self.bounds();
            let w = bounds.size.width;

            // Theme-aware: resolve from the system every draw so dark mode and
            // the accent color follow the user without a cache to invalidate.
            let bar_bg = NSColor::tertiaryLabelColor();
            let bar_fg = NSColor::controlAccentColor();
            let line = NSColor::secondaryLabelColor();

            let bars_h = 28.0;
            let bars_y = bounds.size.height - bars_h;
            for (rect, pct) in bar_rects(w, bars_h, state.core_pcts.len())
                .into_iter()
                .zip(state.core_pcts.iter())
            {
                bar_bg.setFill();
                NSBezierPath::fillRect(NSRect::new(
                    NSPoint::new(rect.x, bars_y),
                    NSSize::new(rect.w, rect.h),
                ));
                bar_fg.setFill();
                let filled = rect.h * (*pct as f64 / 100.0).clamp(0.0, 1.0);
                NSBezierPath::fillRect(NSRect::new(
                    NSPoint::new(rect.x, bars_y),
                    NSSize::new(rect.w, filled),
                ));
            }

            let history: Vec<f32> = state.cpu_history.iter().copied().collect();
            let pts = sparkline_points(&history, w, bounds.size.height - bars_h - 8.0);
            if pts.len() >= 2 {
                let path = NSBezierPath::bezierPath();
                path.moveToPoint(NSPoint::new(pts[0].0, pts[0].1));
                for p in &pts[1..] {
                    path.lineToPoint(NSPoint::new(p.0, p.1));
                }
                path.setLineWidth(1.5);
                line.setStroke();
                path.stroke();
            }
        }
    }
);

impl PanelView {
    pub fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let state = RefCell::new(PanelState {
            core_pcts: Vec::new(),
            cpu_history: Ring::new(60),
        });
        let this = Self::alloc(mtm).set_ivars(state);
        let frame = NSRect::new(
            NSPoint::new(0.0, 0.0),
            NSSize::new(PANEL_WIDTH, PANEL_HEIGHT),
        );
        // SAFETY: `this` is a freshly allocated, not-yet-initialized instance
        // of a class whose superclass is `NSView`, which responds to
        // `initWithFrame:`.
        unsafe { msg_send![super(this), initWithFrame: frame] }
    }

    /// Push a new sample into the view's history and redraw. Called only
    /// while the menu is open (see `Controller::apply_sample`).
    pub fn update(&self, core_pcts: Vec<f32>, cpu_pct: f32) {
        let mut state = self.ivars().borrow_mut();
        state.core_pcts = core_pcts;
        state.cpu_history.push(cpu_pct);
        drop(state);
        self.setNeedsDisplay(true);
    }

    /// Drop the sparkline history. Called on `menuDidClose:` so a closed
    /// dropdown carries no state forward — budget rule 3.
    pub fn clear_history(&self) {
        self.ivars().borrow_mut().cpu_history.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bars_tile_the_width_with_even_gaps() {
        let rects = bar_rects(200.0, 40.0, 4);
        assert_eq!(rects.len(), 4);
        assert!(rects[0].x >= 0.0);
        let right = rects[3].x + rects[3].w;
        assert!(right <= 200.0, "last bar overflows: {right}");
        let gap0 = rects[1].x - (rects[0].x + rects[0].w);
        let gap1 = rects[2].x - (rects[1].x + rects[1].w);
        assert!((gap0 - gap1).abs() < 0.001, "uneven gaps: {gap0} vs {gap1}");
    }

    #[test]
    fn zero_cores_produces_no_bars_instead_of_dividing_by_zero() {
        assert!(bar_rects(200.0, 40.0, 0).is_empty());
    }

    #[test]
    fn sparkline_maps_percentages_into_the_box() {
        let pts = sparkline_points(&[0.0, 50.0, 100.0], 100.0, 20.0);
        assert_eq!(pts.len(), 3);
        assert_eq!(pts[0], (0.0, 0.0));
        assert_eq!(pts[2], (100.0, 20.0));
        assert_eq!(pts[1], (50.0, 10.0));
    }

    #[test]
    fn sparkline_with_one_point_does_not_divide_by_zero() {
        let pts = sparkline_points(&[42.0], 100.0, 20.0);
        assert_eq!(pts, vec![(0.0, 8.4)]);
    }
}
