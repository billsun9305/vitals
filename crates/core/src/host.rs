//! Static machine identity plus the two scheduler-level numbers `macmon`
//! does not report.

use crate::schema::Host;
use macmon::SocInfo;
use sysinfo::System;

/// Copy the static SoC description into the wire type.
pub fn host_from_soc(soc: &SocInfo, ncpu: usize) -> Host {
    Host {
        chip: soc.chip_name.clone(),
        model: soc.mac_model.clone(),
        ecpu_cores: soc.ecpu_cores,
        pcpu_cores: soc.pcpu_cores,
        gpu_cores: soc.gpu_cores,
        ncpu,
    }
}

/// 1 / 5 / 15 minute load averages.
pub fn load_avg() -> [f64; 3] {
    let l = System::load_average();
    [l.one, l.five, l.fifteen]
}

/// Seconds since boot.
pub fn uptime_s() -> u64 {
    System::uptime()
}

/// Logical CPU count — the denominator for load-average saturation.
pub fn ncpu() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_avg_is_three_non_negative_numbers() {
        let l = load_avg();
        assert_eq!(l.len(), 3);
        assert!(l.iter().all(|v| *v >= 0.0), "got {l:?}");
    }

    #[test]
    fn uptime_is_plausible() {
        assert!(uptime_s() > 0, "a running Mac has non-zero uptime");
    }

    #[test]
    fn ncpu_is_at_least_two() {
        assert!(ncpu() >= 2, "every Apple Silicon Mac has >= 2 logical CPUs");
    }

    #[test]
    fn host_from_soc_reads_the_real_machine() {
        let sampler = crate::macmon_sampler_for_test();
        let soc = sampler.get_soc_info();
        let host = host_from_soc(soc, ncpu());
        assert!(host.chip.contains("Apple"), "chip was {:?}", host.chip);
        assert_eq!(host.chip, soc.chip_name);
        assert_eq!(host.model, soc.mac_model);
        assert_eq!(host.ecpu_cores, soc.ecpu_cores);
        assert_eq!(host.pcpu_cores, soc.pcpu_cores);
        assert_eq!(host.gpu_cores, soc.gpu_cores);
        assert_eq!(host.ncpu, ncpu());
    }
}
