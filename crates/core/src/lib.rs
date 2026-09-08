#[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
compile_error!("vitals supports macOS on Apple Silicon (aarch64) only");

pub mod cadence;
pub mod history;
pub mod host;
pub mod pressure;
pub mod procs;
pub mod ring;
pub mod sample;
pub mod schema;
pub mod sysctl;
pub mod thermal;

/// Shared by the hardware-touching tests in several modules.
#[cfg(test)]
pub(crate) fn macmon_sampler_for_test() -> macmon::Sampler {
    macmon::Sampler::new().expect("Sampler::new() failed — are you on Apple Silicon?")
}

#[cfg(test)]
mod tests {
    #[test]
    fn every_declared_module_is_reachable() {
        // The value of this test is linkage: `cargo test` builds the whole
        // module tree, and each assertion below names one module, so a
        // module dropped from lib.rs or emptied stops compiling here rather
        // than failing somewhere confusing later. Each call is the cheapest
        // pure thing that module exposes.
        //
        // (This replaced `assert!(cfg!(target_os = "macos"))`, which was a
        // compile-time constant — always true wherever it could compile at
        // all, since lib.rs already refuses to build off Apple Silicon.)
        use super::*;
        assert_eq!(schema::SCHEMA_VERSION, 1);
        // Pinned, not just referenced: this is the window sysinfo measures
        // per-process CPU over, and `top` reports it as sample_ms.
        assert_eq!(procs::CPU_WINDOW_MS, 200);
        assert_eq!(
            cadence::interval_for(cadence::TrayState {
                menu_open: true,
                on_battery: false,
                low_power: false,
                display_asleep: false,
            }),
            1_000
        );
        assert_eq!(cadence::SAMPLE_WINDOW_MS, 1_000);
        assert!(host::ncpu() >= 1);
        assert!(history::unix_now_s() > 0);
        assert_eq!(pressure::join_with_and(&["a".into()]), "a");
        assert!(!format!("{:?}", sysctl::mem_pressure_level()).is_empty());
        assert!(!format!("{:?}", thermal::thermal_state()).is_empty());
        let mut r: ring::Ring<f32> = ring::Ring::new(1);
        r.push(1.0);
        assert_eq!(r.len(), 1);
        assert!(sample::sample_once(50).is_ok());
    }
}
