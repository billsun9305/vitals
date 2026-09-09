//! Dropdown geometry and drawing.
//!
//! The whole dropdown is one custom view: four sections (CPU, GPU, memory,
//! power), each a header row — label left, headline value right — over a
//! chart. Everything measurable lives in the pure functions below so it can
//! be tested; the `drawRect:` body is only CoreGraphics calls against their
//! output.
//!
//! The view is flipped (origin top-left) so the layout reads top to bottom
//! like the panel does.

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{define_class, msg_send, DefinedClass, MainThreadOnly};
use objc2_app_kit::{
    NSBezierPath, NSColor, NSFont, NSFontAttributeName, NSFontWeightSemibold,
    NSForegroundColorAttributeName, NSStringDrawing, NSView,
};
use objc2_foundation::{MainThreadMarker, NSDictionary, NSPoint, NSRect, NSSize, NSString};
use std::cell::RefCell;
use vitals_core::ring::Ring;
use vitals_core::schema::{CoreKind, PressureLevel, Snapshot};

pub const PANEL_WIDTH: f64 = 300.0;
/// Derived from `layout`, not hand-maintained: see `panel_height_is_the_layout_height`.
pub const PANEL_HEIGHT: f64 = 271.0;

/// Matches the inset NSMenu gives its own text items, so the panel's left
/// edge lines up with "Open Dashboard" below it (measured: 16pt from where
/// the menu places a custom view).
const PAD: f64 = 16.0;
const BAR_GAP: f64 = 3.0;
/// Headline value turns orange here and red at `CRIT_PCT`.
const WARN_PCT: f32 = 70.0;
const CRIT_PCT: f32 = 90.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Rect {
    fn ns(&self) -> NSRect {
        NSRect::new(NSPoint::new(self.x, self.y), NSSize::new(self.w, self.h))
    }
}

/// Where everything sits, in flipped coordinates. Rows are laid out with a
/// running cursor so the total height is a consequence of the content, and
/// adding a row cannot leave the bottom of the panel clipped.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Layout {
    pub width: f64,
    /// Inner column: `PAD` in from each edge.
    pub inner: Rect,
    pub cpu_header: f64,
    pub spark: Rect,
    pub cores: Rect,
    pub legend: f64,
    pub rule1: f64,
    pub gpu_header: f64,
    pub gpu_bar: Rect,
    pub rule2: f64,
    pub mem_header: f64,
    pub mem_bar: Rect,
    pub mem_caption: f64,
    pub rule3: f64,
    pub pwr_header: f64,
    pub height: f64,
}

pub fn layout(width: f64) -> Layout {
    let inner_w = width - 2.0 * PAD;
    let col = |y: f64, h: f64| Rect {
        x: PAD,
        y,
        w: inner_w,
        h,
    };
    const HEADER_H: f64 = 16.0;
    const SECTION_GAP: f64 = 9.0;
    let mut y = 11.0;

    let cpu_header = y;
    y += HEADER_H + 5.0;
    let spark = col(y, 40.0);
    y += spark.h + 8.0;
    let cores = col(y, 20.0);
    y += cores.h + 5.0;
    let legend = y;
    y += 13.0 + SECTION_GAP;
    let rule1 = y;
    y += SECTION_GAP;

    let gpu_header = y;
    y += HEADER_H + 5.0;
    let gpu_bar = col(y, 6.0);
    y += gpu_bar.h + SECTION_GAP;
    let rule2 = y;
    y += SECTION_GAP;

    let mem_header = y;
    y += HEADER_H + 5.0;
    let mem_bar = col(y, 6.0);
    y += mem_bar.h + 5.0;
    let mem_caption = y;
    y += 13.0 + SECTION_GAP;
    let rule3 = y;
    y += SECTION_GAP;

    let pwr_header = y;
    y += HEADER_H + 11.0;

    Layout {
        width,
        inner: col(0.0, y),
        cpu_header,
        spark,
        cores,
        legend,
        rule1,
        gpu_header,
        gpu_bar,
        rule2,
        mem_header,
        mem_bar,
        mem_caption,
        rule3,
        pwr_header,
        height: y,
    }
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
/// on the left. `y` is the distance *up* from the box's bottom edge. A single
/// sample sits at x=0 at its own height.
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

/// How loudly a headline value should be coloured.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Normal,
    Warn,
    Crit,
}

pub fn level(pct: f32) -> Level {
    if pct >= CRIT_PCT {
        Level::Crit
    } else if pct >= WARN_PCT {
        Level::Warn
    } else {
        Level::Normal
    }
}

/// Memory is judged by the kernel's own pressure signal first — that is
/// what actually makes a Mac feel slow — and by the used fraction only as a
/// fallback for when the signal is unavailable.
pub fn mem_level(used_mb: u64, total_mb: u64, pressure: PressureLevel) -> Level {
    match pressure {
        PressureLevel::Critical => Level::Crit,
        PressureLevel::Warning => Level::Warn,
        PressureLevel::Normal => Level::Normal,
        PressureLevel::Unknown => {
            if total_mb == 0 {
                Level::Normal
            } else {
                level(used_mb as f32 / total_mb as f32 * 100.0)
            }
        }
    }
}

/// `27.4` — one decimal, never a unit; the caller places "GB".
pub fn fmt_gb(mb: u64) -> String {
    format!("{:.1}", mb as f64 / 1024.0)
}

/// The strings every header row shows, so they can be asserted on without
/// AppKit. Order matches the sections top to bottom.
pub struct HeaderText {
    pub cpu: String,
    pub cores_legend: String,
    pub gpu: String,
    pub mem: String,
    pub mem_caption: String,
    pub power: String,
}

pub fn header_text(s: &Snapshot) -> HeaderText {
    let mut mem_caption = format!("swap {} GB", fmt_gb(s.swap_used_mb));
    match s.mem_pressure {
        PressureLevel::Warning => mem_caption.push_str("  ·  pressure warning"),
        PressureLevel::Critical => mem_caption.push_str("  ·  pressure critical"),
        PressureLevel::Normal | PressureLevel::Unknown => {}
    }
    HeaderText {
        cpu: format!("{:.1}%", s.cpu_pct),
        cores_legend: format!("E-cores {:.0}%      P-cores {:.0}%", s.ecpu_pct, s.pcpu_pct),
        gpu: format!("{:.1}%  ·  {} MHz", s.gpu_pct, s.gpu_freq_mhz),
        mem: format!("{} / {} GB", fmt_gb(s.mem_used_mb), fmt_gb(s.mem_total_mb)),
        mem_caption,
        power: format!("{:.2} W  ·  {:.0}°C", s.power_total_w, s.temp_cpu_c),
    }
}

/// State backing the custom-drawn dropdown view. Written only while the
/// menu is open (see `Controller::apply_sample` and `menuDidClose:`), so an
/// idle tray never touches it.
pub struct PanelState {
    pub snapshot: Option<Snapshot>,
    /// A sampling failure to show in place of numbers. Cleared by the next
    /// good sample.
    pub error: Option<String>,
    pub cpu_history: Ring<f32>,
}

// ---- drawing helpers ------------------------------------------------------

fn attrs(font: &NSFont, color: &NSColor) -> Retained<NSDictionary<NSString, AnyObject>> {
    // SAFETY: both statics are AppKit-exported constant NSString keys.
    let keys: [&NSString; 2] = unsafe { [NSFontAttributeName, NSForegroundColorAttributeName] };
    // SAFETY: NSFont and NSColor are NSObject subclasses; viewing them as
    // AnyObject for the dictionary is a plain upcast.
    let font: &AnyObject = unsafe { &*(font as *const NSFont as *const AnyObject) };
    let color: &AnyObject = unsafe { &*(color as *const NSColor as *const AnyObject) };
    NSDictionary::from_slices(&keys, &[font, color])
}

fn text_width(s: &NSString, a: &NSDictionary<NSString, AnyObject>) -> f64 {
    // SAFETY: `a` was built by `attrs` with the key types this expects.
    unsafe { s.sizeWithAttributes(Some(a)) }.width
}

fn draw_text(s: &str, x: f64, y: f64, a: &NSDictionary<NSString, AnyObject>) {
    let s = NSString::from_str(s);
    // SAFETY: as above.
    unsafe { s.drawAtPoint_withAttributes(NSPoint::new(x, y), Some(a)) };
}

/// `s` with its right edge at `right`.
fn draw_text_right(s: &str, right: f64, y: f64, a: &NSDictionary<NSString, AnyObject>) {
    let ns = NSString::from_str(s);
    let w = text_width(&ns, a);
    // SAFETY: as above.
    unsafe { ns.drawAtPoint_withAttributes(NSPoint::new(right - w, y), Some(a)) };
}

/// `s` wrapped inside `r`; whatever does not fit is clipped, not overflowed.
fn draw_text_in(s: &str, r: Rect, a: &NSDictionary<NSString, AnyObject>) {
    let s = NSString::from_str(s);
    // SAFETY: as above.
    unsafe { s.drawInRect_withAttributes(r.ns(), Some(a)) };
}

fn fill_rounded(r: Rect, radius: f64, color: &NSColor) {
    color.setFill();
    NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(r.ns(), radius, radius).fill();
}

fn hairline(y: f64, x0: f64, x1: f64) {
    NSColor::separatorColor().setFill();
    NSBezierPath::fillRect(NSRect::new(NSPoint::new(x0, y), NSSize::new(x1 - x0, 1.0)));
}

fn level_color(l: Level) -> Retained<NSColor> {
    match l {
        Level::Normal => NSColor::labelColor(),
        Level::Warn => NSColor::systemOrangeColor(),
        Level::Crit => NSColor::systemRedColor(),
    }
}

/// A horizontal track with `frac` of it filled from the left.
fn track(r: Rect, frac: f64, fill: &NSColor) {
    fill_rounded(r, r.h / 2.0, &NSColor::quaternaryLabelColor());
    let w = r.w * frac.clamp(0.0, 1.0);
    if w > 0.0 {
        fill_rounded(Rect { w, ..r }, r.h / 2.0, fill);
    }
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
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }

        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, _dirty: NSRect) {
            let state = self.ivars().borrow();
            let l = layout(self.bounds().size.width);

            // Theme-aware: resolve from the system every draw so dark mode
            // and the accent colour follow the user without a cache to
            // invalidate.
            let label = NSColor::secondaryLabelColor();
            let accent = NSColor::controlAccentColor();
            let teal = NSColor::systemTealColor();

            let label_font = NSFont::systemFontOfSize_weight(11.0, unsafe { NSFontWeightSemibold });
            let value_font =
                NSFont::monospacedDigitSystemFontOfSize_weight(13.0, unsafe { NSFontWeightSemibold });
            let small_font = NSFont::systemFontOfSize(10.0);
            let label_a = attrs(&label_font, &label);
            let small_a = attrs(&small_font, &label);

            let left = l.inner.x;
            let right = l.inner.x + l.inner.w;
            let value_a = |color: &NSColor| attrs(&value_font, color);

            // The skeleton is drawn in every state — labels, rules, empty
            // tracks, gridlines — so the panel has the same shape before the
            // first sample, during a failure, and live. Numbers arrive into
            // a layout that is already there; nothing jumps.
            let sp = l.spark;
            hairline(sp.y + sp.h, sp.x, sp.x + sp.w);
            hairline(l.rule1, left, right);
            hairline(l.rule2, left, right);
            hairline(l.rule3, left, right);
            draw_text("CPU", left, l.cpu_header, &label_a);
            draw_text("GPU", left, l.gpu_header, &label_a);
            draw_text("MEMORY", left, l.mem_header, &label_a);
            draw_text("POWER", left, l.pwr_header, &label_a);

            // Nothing to number yet — first open, or a sampling failure. Say
            // which, in the space the CPU chart would occupy; the rest of the
            // panel stays as empty tracks.
            let s = match (state.error.as_deref(), state.snapshot.as_ref()) {
                (None, Some(s)) => s,
                (error, _) => {
                    let dim = value_a(&NSColor::tertiaryLabelColor());
                    for y in [l.cpu_header, l.gpu_header, l.mem_header, l.pwr_header] {
                        draw_text_right("—", right, y - 1.0, &dim);
                    }
                    track(l.gpu_bar, 0.0, &accent);
                    track(l.mem_bar, 0.0, &accent);
                    let area = Rect {
                        x: sp.x,
                        y: sp.y + 4.0,
                        w: sp.w,
                        h: (l.legend + 13.0) - (sp.y + 4.0),
                    };
                    match error {
                        Some(err) => draw_text_in(
                            &format!("⚠  {err}"),
                            area,
                            &attrs(&label_font, &NSColor::systemOrangeColor()),
                        ),
                        None => draw_text("Sampling…", left, sp.y + 4.0, &label_a),
                    }
                    return;
                }
            };
            let t = header_text(s);

            // ---- CPU -------------------------------------------------------
            draw_text_right(
                &t.cpu,
                right,
                l.cpu_header - 1.0,
                &value_a(&level_color(level(s.cpu_pct))),
            );

            // Sparkline: quarter gridlines, an area fill under the line, then
            // the line. Flipped view, so a percentage maps to a distance up
            // from the box's bottom edge. The gridlines are drawn here and
            // not in the skeleton because the placeholder text sits in this
            // box, and lines through letters look like a defect.
            NSColor::quaternaryLabelColor().setFill();
            for q in [0.25, 0.5, 0.75] {
                let y = sp.y + sp.h * (1.0 - q);
                NSBezierPath::fillRect(NSRect::new(
                    NSPoint::new(sp.x, y.round()),
                    NSSize::new(sp.w, 1.0),
                ));
            }
            let history: Vec<f32> = state.cpu_history.iter().copied().collect();
            let pts = sparkline_points(&history, sp.w, sp.h);
            if pts.len() >= 2 {
                let bottom = sp.y + sp.h;
                let at = |p: &(f64, f64)| NSPoint::new(sp.x + p.0, bottom - p.1);
                let area = NSBezierPath::bezierPath();
                area.moveToPoint(NSPoint::new(sp.x + pts[0].0, bottom));
                for p in &pts {
                    area.lineToPoint(at(p));
                }
                area.lineToPoint(NSPoint::new(sp.x + pts[pts.len() - 1].0, bottom));
                area.closePath();
                accent.colorWithAlphaComponent(0.18).setFill();
                area.fill();

                let line = NSBezierPath::bezierPath();
                line.moveToPoint(at(&pts[0]));
                for p in &pts[1..] {
                    line.lineToPoint(at(p));
                }
                line.setLineWidth(1.5);
                accent.setStroke();
                line.stroke();
            }

            // Per-core bars, E-cores tinted differently from P-cores so a
            // pair pinned at 100% reads as "background work on the efficient
            // cores", not as an alarm.
            let cr = l.cores;
            for (r, core) in bar_rects(cr.w, cr.h, s.cores.len())
                .into_iter()
                .zip(s.cores.iter())
            {
                let r = Rect {
                    x: cr.x + r.x,
                    y: cr.y,
                    ..r
                };
                fill_rounded(r, 2.0, &NSColor::quaternaryLabelColor());
                let filled = r.h * (core.pct as f64 / 100.0).clamp(0.0, 1.0);
                if filled > 0.0 {
                    let fill = Rect {
                        y: r.y + r.h - filled,
                        h: filled,
                        ..r
                    };
                    let color = match core.kind {
                        CoreKind::E => &teal,
                        CoreKind::P => &accent,
                    };
                    fill_rounded(fill, 2.0, color);
                }
            }

            // Legend: a swatch of each tint before its label.
            let swatch = |x: f64, color: &NSColor| {
                fill_rounded(
                    Rect {
                        x,
                        y: l.legend + 3.0,
                        w: 7.0,
                        h: 7.0,
                    },
                    2.0,
                    color,
                );
            };
            let e_label = format!("E-cores {:.0}%", s.ecpu_pct);
            let p_label = format!("P-cores {:.0}%", s.pcpu_pct);
            swatch(left, &teal);
            draw_text(&e_label, left + 11.0, l.legend, &small_a);
            let e_w = text_width(&NSString::from_str(&e_label), &small_a);
            let px = left + 11.0 + e_w + 14.0;
            swatch(px, &accent);
            draw_text(&p_label, px + 11.0, l.legend, &small_a);

            // ---- GPU -------------------------------------------------------
            draw_text_right(
                &t.gpu,
                right,
                l.gpu_header - 1.0,
                &value_a(&level_color(level(s.gpu_pct))),
            );
            track(l.gpu_bar, s.gpu_pct as f64 / 100.0, &accent);

            // ---- MEMORY ----------------------------------------------------
            let ml = mem_level(s.mem_used_mb, s.mem_total_mb, s.mem_pressure);
            draw_text_right(&t.mem, right, l.mem_header - 1.0, &value_a(&level_color(ml)));
            let frac = if s.mem_total_mb == 0 {
                0.0
            } else {
                s.mem_used_mb as f64 / s.mem_total_mb as f64
            };
            let mem_fill = match ml {
                Level::Normal => accent.clone(),
                other => level_color(other),
            };
            track(l.mem_bar, frac, &mem_fill);
            let caption_color = match ml {
                Level::Normal => label.clone(),
                other => level_color(other),
            };
            draw_text(
                &t.mem_caption,
                left,
                l.mem_caption,
                &attrs(&small_font, &caption_color),
            );

            // ---- POWER -----------------------------------------------------
            draw_text_right(
                &t.power,
                right,
                l.pwr_header - 1.0,
                &value_a(&NSColor::labelColor()),
            );
        }
    }
);

impl PanelView {
    pub fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let state = RefCell::new(PanelState {
            snapshot: None,
            error: None,
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

    /// Show a snapshot without adding it to the history. Used when the menu
    /// opens, to paint what the tray already knows rather than wait a tick —
    /// pushing there would duplicate the most recent point.
    pub fn show(&self, snapshot: &Snapshot) {
        let mut state = self.ivars().borrow_mut();
        state.snapshot = Some(snapshot.clone());
        state.error = None;
        drop(state);
        self.setNeedsDisplay(true);
    }

    /// Record a fresh sample: show it and extend the sparkline. Called only
    /// while the menu is open (see `Controller::apply_sample`).
    pub fn push_sample(&self, snapshot: &Snapshot) {
        let mut state = self.ivars().borrow_mut();
        state.cpu_history.push(snapshot.cpu_pct);
        state.snapshot = Some(snapshot.clone());
        state.error = None;
        drop(state);
        self.setNeedsDisplay(true);
    }

    /// Replace the numbers with a sampling failure until the next good
    /// sample clears it.
    pub fn show_error(&self, message: &str) {
        self.ivars().borrow_mut().error = Some(message.to_string());
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
    use vitals_core::schema::{Core, Host, ThermalState, SCHEMA_VERSION};

    fn snap() -> Snapshot {
        let core = |id: usize, kind: CoreKind, pct: f32| Core {
            id,
            kind,
            die_id: 0,
            core_id: id,
            pct,
            freq_mhz: 2064,
        };
        Snapshot {
            schema_version: SCHEMA_VERSION,
            sampled_at: "2026-09-09T04:00:00Z".into(),
            sample_ms: 1000,
            host: Host {
                chip: "Apple M1 Pro".into(),
                model: "MacBookPro18,3".into(),
                ecpu_cores: 2,
                pcpu_cores: 8,
                gpu_cores: 14,
                ncpu: 10,
            },
            cpu_pct: 29.3,
            cpu_scaled_pct: 28.0,
            ecpu_pct: 100.0,
            ecpu_freq_mhz: 2064,
            pcpu_pct: 11.6,
            pcpu_freq_mhz: 3200,
            cores: vec![
                core(0, CoreKind::E, 100.0),
                core(1, CoreKind::E, 100.0),
                core(2, CoreKind::P, 11.6),
            ],
            gpu_pct: 29.4,
            gpu_freq_mhz: 410,
            mem_total_mb: 32_768,
            mem_used_mb: 27_370,
            swap_total_mb: 19_456,
            swap_used_mb: 17_600,
            mem_pressure: PressureLevel::Warning,
            power_total_w: 2.13,
            power_cpu_w: 1.5,
            power_gpu_w: 0.4,
            power_ane_w: 0.0,
            power_ram_w: 0.1,
            power_sys_w: 2.13,
            temp_cpu_c: 53.4,
            temp_gpu_c: 50.0,
            fans: Vec::new(),
            load_avg: [1.0, 1.0, 1.0],
            uptime_s: 1,
            thermal_state: ThermalState::Nominal,
        }
    }

    #[test]
    fn panel_height_is_the_layout_height() {
        // The constant exists because `initWithFrame:` needs a number before
        // the first draw; this is what keeps it honest.
        assert_eq!(layout(PANEL_WIDTH).height, PANEL_HEIGHT);
    }

    #[test]
    fn layout_reads_top_to_bottom_and_stays_inside_the_panel() {
        let l = layout(PANEL_WIDTH);
        let ys = [
            l.cpu_header,
            l.spark.y,
            l.spark.y + l.spark.h,
            l.cores.y,
            l.cores.y + l.cores.h,
            l.legend,
            l.rule1,
            l.gpu_header,
            l.gpu_bar.y,
            l.rule2,
            l.mem_header,
            l.mem_bar.y,
            l.mem_caption,
            l.rule3,
            l.pwr_header,
        ];
        for w in ys.windows(2) {
            assert!(w[0] < w[1], "rows out of order: {} then {}", w[0], w[1]);
        }
        assert!(l.pwr_header + 16.0 <= l.height, "power row clipped");
        for r in [l.spark, l.cores, l.gpu_bar, l.mem_bar] {
            assert!(
                r.x >= PAD && r.x + r.w <= PANEL_WIDTH - PAD,
                "{r:?} breaks the margin"
            );
        }
    }

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

    #[test]
    fn headline_values_escalate_at_seventy_and_ninety() {
        assert_eq!(level(69.9), Level::Normal);
        assert_eq!(level(70.0), Level::Warn);
        assert_eq!(level(89.9), Level::Warn);
        assert_eq!(level(90.0), Level::Crit);
    }

    #[test]
    fn memory_trusts_the_kernel_pressure_signal_over_the_used_fraction() {
        // 20% used but the kernel says warning: warning wins. This is the
        // case a compressed-memory Mac actually presents.
        assert_eq!(
            mem_level(6_000, 32_768, PressureLevel::Warning),
            Level::Warn
        );
        assert_eq!(
            mem_level(6_000, 32_768, PressureLevel::Critical),
            Level::Crit
        );
        // 95% used but the kernel is fine: fine.
        assert_eq!(
            mem_level(31_000, 32_768, PressureLevel::Normal),
            Level::Normal
        );
        // No signal: fall back to the fraction.
        assert_eq!(
            mem_level(31_000, 32_768, PressureLevel::Unknown),
            Level::Crit
        );
        assert_eq!(mem_level(1, 0, PressureLevel::Unknown), Level::Normal);
    }

    #[test]
    fn header_text_reports_every_headline_number() {
        let t = header_text(&snap());
        assert_eq!(t.cpu, "29.3%");
        assert_eq!(t.cores_legend, "E-cores 100%      P-cores 12%");
        assert_eq!(t.gpu, "29.4%  ·  410 MHz");
        assert_eq!(t.mem, "26.7 / 32.0 GB");
        assert_eq!(t.mem_caption, "swap 17.2 GB  ·  pressure warning");
        assert_eq!(t.power, "2.13 W  ·  53°C");
    }

    #[test]
    fn memory_caption_is_silent_when_pressure_is_normal() {
        let mut s = snap();
        s.mem_pressure = PressureLevel::Normal;
        assert_eq!(header_text(&s).mem_caption, "swap 17.2 GB");
    }
}
