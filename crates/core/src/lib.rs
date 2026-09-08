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
    fn every_declared_module_compiles() {
        // The value of this test is that `cargo test` builds the full module
        // tree; parallel tasks each fill in one stub and must not have to
        // touch lib.rs.
        assert!(cfg!(all(target_os = "macos", target_arch = "aarch64")));
    }
}
