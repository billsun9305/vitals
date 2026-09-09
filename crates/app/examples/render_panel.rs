//! Render the dropdown panel to PNG, light and dark, without a menu bar.
//!
//! The panel can only be seen by opening a live menu on a machine with a
//! screen, which is a slow loop and one the person doing the drawing cannot
//! close without a screen-recording grant. This paints the same `drawRect:`
//! into an offscreen window instead:
//!
//!     cargo run --release --example render_panel -- /tmp/out
//!
//! writes `/tmp/out/panel-light.png` and `panel-dark.png` at 2x.

use objc2::rc::Retained;
use objc2::runtime::NSObjectProtocol;
use objc2::{define_class, msg_send, MainThreadOnly};
use objc2_app_kit::{
    NSAppearance, NSAppearanceCustomization, NSAppearanceNameAqua, NSAppearanceNameDarkAqua,
    NSApplication, NSBackingStoreType, NSBezierPath, NSBitmapImageFileType, NSColor, NSView,
    NSWindow, NSWindowStyleMask,
};
use objc2_foundation::{MainThreadMarker, NSDictionary, NSPoint, NSRect, NSSize, NSString};
use vitals::tray::panel::{PanelView, PANEL_HEIGHT, PANEL_WIDTH};
use vitals_core::schema::{
    Core, CoreKind, Host, PressureLevel, Snapshot, ThermalState, SCHEMA_VERSION,
};

fn snapshot(cpu: f32) -> Snapshot {
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
        cpu_pct: cpu,
        cpu_scaled_pct: cpu,
        ecpu_pct: 100.0,
        ecpu_freq_mhz: 2064,
        pcpu_pct: 11.6,
        pcpu_freq_mhz: 3200,
        cores: vec![
            core(0, CoreKind::E, 100.0),
            core(1, CoreKind::E, 100.0),
            core(2, CoreKind::P, 38.0),
            core(3, CoreKind::P, 22.0),
            core(4, CoreKind::P, 9.0),
            core(5, CoreKind::P, 14.0),
            core(6, CoreKind::P, 6.0),
            core(7, CoreKind::P, 3.0),
            core(8, CoreKind::P, 1.0),
            core(9, CoreKind::P, 0.5),
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

define_class!(
    /// Paints the menu's backdrop behind the panel. In a real menu AppKit
    /// draws this; offscreen nothing does, and dark mode's white text on a
    /// transparent PNG is invisible.
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[name = "VitalsRenderBackdrop"]
    struct Backdrop;

    unsafe impl NSObjectProtocol for Backdrop {}

    impl Backdrop {
        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, _dirty: NSRect) {
            NSColor::windowBackgroundColor().setFill();
            NSBezierPath::fillRect(self.bounds());
        }
    }
);

/// Which of the panel's states to paint.
#[derive(Clone, Copy)]
enum Scene {
    Live,
    Sampling,
    Error,
}

fn render(mtm: MainThreadMarker, appearance: &NSAppearance, scene: Scene, path: &str) {
    let view = PanelView::new(mtm);
    // A plausible minute of history: a bump, a lull, a climb.
    let series = [
        22.0, 25.0, 31.0, 48.0, 62.0, 55.0, 41.0, 33.0, 28.0, 26.0, 24.0, 23.0, 27.0, 35.0, 44.0,
        39.0, 30.0, 29.0, 31.0, 34.0, 38.0, 29.3,
    ];
    match scene {
        Scene::Live => {
            for v in series {
                view.push_sample(&snapshot(v));
            }
        }
        Scene::Sampling => {}
        Scene::Error => view.show_error("IOReport: subscription failed (kIOReturnNotPermitted)"),
    }

    let rect = NSRect::new(
        NSPoint::new(0.0, 0.0),
        NSSize::new(PANEL_WIDTH, PANEL_HEIGHT),
    );
    // An offscreen window gives the view a backing store at the screen's
    // scale factor, so the PNG is 2x on a Retina Mac and text is legible.
    let window: Retained<NSWindow> = unsafe {
        NSWindow::initWithContentRect_styleMask_backing_defer(
            NSWindow::alloc(mtm),
            rect,
            NSWindowStyleMask::Borderless,
            NSBackingStoreType::Buffered,
            false,
        )
    };
    window.setAppearance(Some(appearance));
    let backdrop: Retained<Backdrop> =
        unsafe { msg_send![Backdrop::alloc(mtm), initWithFrame: rect] };
    backdrop.addSubview(&view);
    window.setContentView(Some(&backdrop));

    let rep = backdrop
        .bitmapImageRepForCachingDisplayInRect(rect)
        .expect("bitmap rep");
    backdrop.cacheDisplayInRect_toBitmapImageRep(rect, &rep);
    // SAFETY: an empty properties dictionary is valid for PNG output.
    let png = unsafe {
        rep.representationUsingType_properties(NSBitmapImageFileType::PNG, &NSDictionary::new())
    }
    .expect("png");
    assert!(
        png.writeToFile_atomically(&NSString::from_str(path), true),
        "write {path}"
    );
    println!("{path}  ({}x{} px)", rep.pixelsWide(), rep.pixelsHigh());
}

fn main() {
    let dir = std::env::args().nth(1).unwrap_or_else(|| ".".into());
    std::fs::create_dir_all(&dir).expect("out dir");
    let mtm = MainThreadMarker::new().expect("main thread");
    let _app = NSApplication::sharedApplication(mtm);
    for (name, scene, file) in [
        (
            unsafe { NSAppearanceNameAqua },
            Scene::Live,
            "panel-light.png",
        ),
        (
            unsafe { NSAppearanceNameDarkAqua },
            Scene::Live,
            "panel-dark.png",
        ),
        (
            unsafe { NSAppearanceNameAqua },
            Scene::Sampling,
            "panel-sampling.png",
        ),
        (
            unsafe { NSAppearanceNameDarkAqua },
            Scene::Error,
            "panel-error-dark.png",
        ),
    ] {
        let appearance = NSAppearance::appearanceNamed(name).expect("appearance");
        render(mtm, &appearance, scene, &format!("{dir}/{file}"));
    }
}
