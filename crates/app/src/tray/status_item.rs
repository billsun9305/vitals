//! Everything the menu bar displays, as pure functions over a `Snapshot`.
//!
//! Kept free of AppKit deliberately: a rendered status item cannot be
//! asserted on from `cargo test`, but the strings that go into it can.

use vitals_core::schema::Snapshot;

/// The menu bar string. Text, not a graph: it is redrawn only when it
/// changes, which is what keeps the idle cost near zero.
pub fn format_title(s: &Snapshot) -> String {
    let gb = s.mem_used_mb as f32 / 1024.0;
    // 99.95, not 100.0: the else branch formats with {:.1}, which rounds up,
    // so anything from 99.95 GB renders as "100.0G" and blows the width
    // budget by a character. Branch on what will be *printed*, not on the
    // value.
    if gb >= 99.95 {
        // Three significant digits are enough, and the decimal would push
        // the item past its width budget on a 128 GB machine.
        format!("{:.0}% · {:.0}G", s.cpu_pct, gb)
    } else {
        format!("{:.0}% · {:.1}G", s.cpu_pct, gb)
    }
}

/// The four informational rows of the dropdown, in order.
pub fn menu_lines(s: &Snapshot) -> [String; 4] {
    [
        format!(
            "CPU  {:.1}%   E {:.1}%   P {:.1}%",
            s.cpu_pct, s.ecpu_pct, s.pcpu_pct
        ),
        format!("GPU  {:.1}%   {} MHz", s.gpu_pct, s.gpu_freq_mhz),
        format!(
            "MEM  {} / {} MB   swap {} MB",
            s.mem_used_mb, s.mem_total_mb, s.swap_used_mb
        ),
        format!("PWR  {:.2} W   {:.1}°C", s.power_total_w, s.temp_cpu_c),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use vitals_core::schema::{Host, PressureLevel, ThermalState, SCHEMA_VERSION};

    /// A zeroed `Snapshot` with the two fields the title depends on set.
    ///
    /// The brief built this through `build_snapshot(SnapshotInputs { .. })`,
    /// which needs `macmon` as a dev-dependency of this crate. The manifest
    /// is off-limits for this task, and a struct literal is in any case the
    /// tighter unit test: it pins the exact inputs `format_title` reads
    /// without routing them through another function's rounding.
    fn snap(cpu: f32, mem_used: u64) -> Snapshot {
        Snapshot {
            schema_version: SCHEMA_VERSION,
            sampled_at: "2026-09-07T22:04:05Z".into(),
            sample_ms: 1000,
            host: Host {
                chip: "Apple M3 Pro".into(),
                model: "Mac15,6".into(),
                ecpu_cores: 6,
                pcpu_cores: 6,
                gpu_cores: 18,
                ncpu: 12,
            },
            cpu_pct: cpu,
            cpu_scaled_pct: 0.0,
            ecpu_pct: 0.0,
            ecpu_freq_mhz: 0,
            pcpu_pct: 0.0,
            pcpu_freq_mhz: 0,
            cores: Vec::new(),
            gpu_pct: 0.0,
            gpu_freq_mhz: 0,
            mem_total_mb: 36_864,
            mem_used_mb: mem_used,
            swap_total_mb: 0,
            swap_used_mb: 0,
            mem_pressure: PressureLevel::Normal,
            power_total_w: 0.0,
            power_cpu_w: 0.0,
            power_gpu_w: 0.0,
            power_ane_w: 0.0,
            power_ram_w: 0.0,
            power_sys_w: 0.0,
            temp_cpu_c: 0.0,
            temp_gpu_c: 0.0,
            fans: Vec::new(),
            load_avg: [1.0, 1.0, 1.0],
            uptime_s: 1,
            thermal_state: ThermalState::Nominal,
        }
    }

    #[test]
    fn title_is_short_and_fixed_width() {
        assert_eq!(format_title(&snap(12.4, 18_211)), "12% · 17.8G");
        assert_eq!(format_title(&snap(100.0, 4_096)), "100% · 4.0G");
        assert_eq!(format_title(&snap(0.0, 512)), "0% · 0.5G");
    }

    #[test]
    fn large_memory_drops_the_decimal_to_stay_short() {
        // Menu bar real estate is contested; a long title pushes other items
        // off. A 128 GB Mac Studio is the widest realistic case.
        assert_eq!(format_title(&snap(100.0, 131_072)), "100% · 128G");
    }

    #[test]
    fn title_never_exceeds_twelve_characters() {
        // A handful of hand-picked sizes cannot find a rounding boundary, so
        // sweep instead: every 7 MB from 0 to ~195 GB, at the widest CPU
        // string there is. The prime step keeps the samples from landing
        // only on round GB values, which is exactly where the bug wasn't.
        for mb in (0..=200_000).step_by(7) {
            let t = format_title(&snap(100.0, mb));
            assert!(t.chars().count() <= 12, "title too long at {mb} MB: {t:?}");
        }
    }

    #[test]
    fn the_decimal_is_dropped_before_the_hundred_gigabyte_mark() {
        // 99.95 GB and up print as "100.0G" under {:.1}, one character over
        // budget, so the branch has to fire below 100 GB, not at it.
        assert_eq!(format_title(&snap(100.0, 102_348)), "100% · 99.9G");
        assert_eq!(format_title(&snap(100.0, 102_349)), "100% · 100G");
    }

    #[test]
    fn menu_lines_report_every_headline_number() {
        let mut s = snap(12.4, 18_211);
        s.ecpu_pct = 8.1;
        s.pcpu_pct = 21.0;
        s.gpu_pct = 4.0;
        s.gpu_freq_mhz = 444;
        s.swap_used_mb = 2_048;
        s.power_total_w = 8.12;
        s.temp_cpu_c = 51.5;
        let lines = menu_lines(&s);
        assert_eq!(lines[0], "CPU  12.4%   E 8.1%   P 21.0%");
        assert_eq!(lines[1], "GPU  4.0%   444 MHz");
        assert_eq!(lines[2], "MEM  18211 / 36864 MB   swap 2048 MB");
        assert_eq!(lines[3], "PWR  8.12 W   51.5°C");
    }
}
