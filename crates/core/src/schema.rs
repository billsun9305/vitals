//! Every serde type `vitals` emits. This module is the agent contract:
//! adding a field is free, renaming or removing one is a breaking change
//! that must bump `SCHEMA_VERSION`.

use serde::{Deserialize, Serialize};
use time::{format_description::well_known::Rfc3339, OffsetDateTime};

/// Version of the JSON contract emitted by every command.
pub const SCHEMA_VERSION: u32 = 1;

/// Current time as an RFC3339 UTC timestamp, second precision.
///
/// Deliberately UTC: `OffsetDateTime::now_local()` fails in multi-threaded
/// processes on Unix, and `vitals` is multi-threaded in three of its four
/// faces. An agent reading `Z` never has to guess an offset.
pub fn now_rfc3339() -> String {
    OffsetDateTime::now_utc()
        .replace_nanosecond(0)
        .unwrap_or_else(|_| OffsetDateTime::now_utc())
        .format(&Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PressureLevel { Normal, Warning, Critical, Unknown }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThermalState { Nominal, Fair, Serious, Critical, Unknown }

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity { Nominal, Warning, Critical }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum CoreKind { E, P }

#[derive(Debug, Clone, Serialize)]
pub struct Host {
    pub chip: String,
    pub model: String,
    pub ecpu_cores: u8,
    pub pcpu_cores: u8,
    pub gpu_cores: u8,
    /// Logical CPUs as the scheduler sees them; the denominator for load average.
    pub ncpu: usize,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct Core {
    /// Index in the concatenation of E cores then P cores.
    pub id: usize,
    pub kind: CoreKind,
    pub die_id: usize,
    pub core_id: usize,
    pub pct: f32,
    pub freq_mhz: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct Fan {
    pub name: String,
    pub rpm: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_rpm: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Snapshot {
    pub schema_version: u32,
    pub sampled_at: String,
    pub sample_ms: u32,
    pub host: Host,
    /// Utilization: fraction of the window any core was active, x100.
    pub cpu_pct: f32,
    /// Work rate: active residency weighted by frequency against the core max, x100.
    pub cpu_scaled_pct: f32,
    pub ecpu_pct: f32,
    pub ecpu_freq_mhz: u32,
    pub pcpu_pct: f32,
    pub pcpu_freq_mhz: u32,
    pub cores: Vec<Core>,
    pub gpu_pct: f32,
    pub gpu_freq_mhz: u32,
    pub mem_total_mb: u64,
    pub mem_used_mb: u64,
    pub swap_total_mb: u64,
    pub swap_used_mb: u64,
    pub mem_pressure: PressureLevel,
    pub power_total_w: f32,
    pub power_cpu_w: f32,
    pub power_gpu_w: f32,
    pub power_ane_w: f32,
    pub power_ram_w: f32,
    pub power_sys_w: f32,
    pub temp_cpu_c: f32,
    pub temp_gpu_c: f32,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub fans: Vec<Fan>,
    pub load_avg: [f64; 3],
    pub uptime_s: u64,
    pub thermal_state: ThermalState,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProcRow {
    pub pid: u32,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app: Option<String>,
    /// Per-process CPU percent. Exceeds 100 on multi-core work, like `ps`.
    pub cpu_pct: f32,
    pub mem_mb: u64,
    pub threads: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct TopReport {
    pub schema_version: u32,
    pub sampled_at: String,
    pub sample_ms: u32,
    pub by_cpu: Vec<ProcRow>,
    pub by_mem: Vec<ProcRow>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Reason {
    pub code: String,
    pub severity: Severity,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Suspect {
    pub pid: u32,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app: Option<String>,
    pub why: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Verdict {
    pub schema_version: u32,
    pub sampled_at: String,
    pub state: Severity,
    /// False when no usable prior observation existed; swap-trend reasons
    /// are then not evaluated and everything else still is.
    pub history: bool,
    pub reasons: Vec<Reason>,
    pub suspects: Vec<Suspect>,
    pub summary: String,
}

fn r1(v: f32) -> f32 { (v * 10.0).round() / 10.0 }
fn r2(v: f32) -> f32 { (v * 100.0).round() / 100.0 }
fn pct(ratio: f32) -> f32 { r1(ratio * 100.0) }
fn mb(bytes: u64) -> u64 { bytes / (1024 * 1024) }

/// Everything `build_snapshot` needs that is not already in `Metrics`.
pub struct SnapshotInputs<'a> {
    pub metrics: &'a macmon::Metrics,
    pub host: Host,
    pub sample_ms: u32,
    pub sampled_at: String,
    pub mem_pressure: PressureLevel,
    pub thermal_state: ThermalState,
    pub load_avg: [f64; 3],
    pub uptime_s: u64,
}

/// Pure mapping from `macmon`'s raw sample to the wire schema. No I/O, no
/// clock reads — every varying input arrives through `SnapshotInputs`, which
/// is what makes this exhaustively testable.
pub fn build_snapshot(i: SnapshotInputs<'_>) -> Snapshot {
    let m = i.metrics;

    let mut cores = Vec::with_capacity(m.ecpu_cores.len() + m.pcpu_cores.len());
    for (kind, list) in [(CoreKind::E, &m.ecpu_cores), (CoreKind::P, &m.pcpu_cores)] {
        for c in list.iter() {
            cores.push(Core {
                id: cores.len(),
                kind,
                die_id: c.die_id,
                core_id: c.core_id,
                pct: pct(c.active_ratio),
                freq_mhz: c.freq_mhz,
            });
        }
    }

    Snapshot {
        schema_version: SCHEMA_VERSION,
        sampled_at: i.sampled_at,
        sample_ms: i.sample_ms,
        host: i.host,
        cpu_pct: pct(m.cpu_active_ratio),
        cpu_scaled_pct: pct(m.cpu_scaled_ratio),
        ecpu_pct: pct(m.ecpu_active_ratio),
        ecpu_freq_mhz: m.ecpu_freq_mhz,
        pcpu_pct: pct(m.pcpu_active_ratio),
        pcpu_freq_mhz: m.pcpu_freq_mhz,
        cores,
        gpu_pct: pct(m.gpu_active_ratio),
        gpu_freq_mhz: m.gpu_freq_mhz,
        mem_total_mb: mb(m.memory.ram_total),
        mem_used_mb: mb(m.memory.ram_usage),
        swap_total_mb: mb(m.memory.swap_total),
        swap_used_mb: mb(m.memory.swap_usage),
        mem_pressure: i.mem_pressure,
        power_total_w: r2(m.all_power),
        power_cpu_w: r2(m.cpu_power),
        power_gpu_w: r2(m.gpu_power),
        power_ane_w: r2(m.ane_power),
        power_ram_w: r2(m.ram_power),
        power_sys_w: r2(m.sys_power),
        temp_cpu_c: r1(m.temp.cpu_temp_avg),
        temp_gpu_c: r1(m.temp.gpu_temp_avg),
        fans: m.fans.iter().map(|f| Fan {
            name: f.name.clone(),
            rpm: f.rpm,
            max_rpm: f.max_rpm,
        }).collect(),
        load_avg: i.load_avg,
        uptime_s: i.uptime_s,
        thermal_state: i.thermal_state,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inputs_for(m: &macmon::Metrics) -> SnapshotInputs<'_> {
        SnapshotInputs {
            metrics: m,
            host: Host {
                chip: "Apple M3 Pro".into(), model: "Mac15,6".into(),
                ecpu_cores: 6, pcpu_cores: 6, gpu_cores: 18, ncpu: 12,
            },
            sample_ms: 200,
            sampled_at: "2026-09-07T22:04:05Z".into(),
            mem_pressure: PressureLevel::Normal,
            thermal_state: ThermalState::Nominal,
            load_avg: [2.14, 1.98, 1.75],
            uptime_s: 419203,
        }
    }

    #[test]
    fn default_metrics_produce_a_valid_zeroed_snapshot() {
        let m = macmon::Metrics::default();
        let s = build_snapshot(inputs_for(&m));
        assert_eq!(s.schema_version, 1);
        assert_eq!(s.sample_ms, 200);
        assert_eq!(s.cpu_pct, 0.0);
        assert!(s.cores.is_empty());
        assert!(s.fans.is_empty());
        assert_eq!(s.mem_total_mb, 0);
    }

    #[test]
    fn bytes_become_megabytes() {
        let mut m = macmon::Metrics::default();
        m.memory.ram_total = 36 * 1024 * 1024 * 1024;
        m.memory.ram_usage = 18 * 1024 * 1024 * 1024;
        m.memory.swap_total = 6 * 1024 * 1024 * 1024;
        m.memory.swap_usage = 2 * 1024 * 1024 * 1024;
        let s = build_snapshot(inputs_for(&m));
        assert_eq!(s.mem_total_mb, 36864);
        assert_eq!(s.mem_used_mb, 18432);
        assert_eq!(s.swap_total_mb, 6144);
        assert_eq!(s.swap_used_mb, 2048);
    }

    #[test]
    fn ratios_become_percentages_rounded_to_one_decimal() {
        let m = macmon::Metrics {
            cpu_active_ratio: 0.1239,
            cpu_scaled_ratio: 0.0412,
            gpu_active_ratio: 0.04,
            ..Default::default()
        };
        let s = build_snapshot(inputs_for(&m));
        assert_eq!(s.cpu_pct, 12.4);
        assert_eq!(s.cpu_scaled_pct, 4.1);
        assert_eq!(s.gpu_pct, 4.0);
    }

    #[test]
    fn cores_are_e_then_p_with_a_dense_id() {
        let m = macmon::Metrics {
            ecpu_cores: vec![
            macmon::CpuCoreMetrics { die_id: 0, core_id: 0, freq_mhz: 1104, active_ratio: 0.062, scaled_ratio: 0.02 },
            macmon::CpuCoreMetrics { die_id: 0, core_id: 1, freq_mhz: 1200, active_ratio: 0.10,  scaled_ratio: 0.04 },
        ],
            pcpu_cores: vec![
            macmon::CpuCoreMetrics { die_id: 0, core_id: 0, freq_mhz: 3204, active_ratio: 0.50, scaled_ratio: 0.40 },
        ],
            ..Default::default()
        };
        let s = build_snapshot(inputs_for(&m));
        let ids: Vec<usize> = s.cores.iter().map(|c| c.id).collect();
        let kinds: Vec<CoreKind> = s.cores.iter().map(|c| c.kind).collect();
        assert_eq!(ids, vec![0, 1, 2]);
        assert_eq!(kinds, vec![CoreKind::E, CoreKind::E, CoreKind::P]);
        assert_eq!(s.cores[0].pct, 6.2);
        assert_eq!(s.cores[2].freq_mhz, 3204);
        assert_eq!(s.cores[2].core_id, 0, "core_id is per-cluster and is passed through unchanged");
    }

    #[test]
    fn fans_pass_through_and_omit_absent_max_rpm() {
        let m = macmon::Metrics {
            fans: vec![
            macmon::FanMetric { name: "fan0".into(), rpm: 1820, max_rpm: Some(4400) },
            macmon::FanMetric { name: "fan1".into(), rpm: 1790, max_rpm: None },
        ],
            ..Default::default()
        };
        let s = build_snapshot(inputs_for(&m));
        let json = serde_json::to_value(&s).unwrap();
        assert_eq!(json["fans"][0]["max_rpm"], 4400);
        assert!(json["fans"][1].get("max_rpm").is_none());
        assert_eq!(json["fans"][1]["name"], "fan1");
    }

    #[test]
    fn watts_round_to_two_decimals() {
        let m = macmon::Metrics {
            all_power: 8.1234,
            cpu_power: 3.4051,
            ..Default::default()
        };
        let s = build_snapshot(inputs_for(&m));
        assert_eq!(s.power_total_w, 8.12);
        assert_eq!(s.power_cpu_w, 3.41);
    }

    #[test]
    fn enums_serialize_lowercase() {
        assert_eq!(serde_json::to_string(&PressureLevel::Warning).unwrap(), "\"warning\"");
        assert_eq!(serde_json::to_string(&ThermalState::Serious).unwrap(), "\"serious\"");
        assert_eq!(serde_json::to_string(&Severity::Critical).unwrap(), "\"critical\"");
        assert_eq!(serde_json::to_string(&CoreKind::E).unwrap(), "\"E\"");
    }

    #[test]
    fn severity_orders_nominal_below_critical() {
        assert!(Severity::Nominal < Severity::Warning);
        assert!(Severity::Warning < Severity::Critical);
    }

    #[test]
    fn optional_fields_are_omitted_never_null() {
        let fan = Fan { name: "fan0".into(), rpm: 1820, max_rpm: None };
        let json = serde_json::to_string(&fan).unwrap();
        assert_eq!(json, r#"{"name":"fan0","rpm":1820}"#);

        let row = ProcRow { pid: 1, name: "launchd".into(), app: None, cpu_pct: 0.1, mem_mb: 12, threads: 4 };
        assert!(!serde_json::to_string(&row).unwrap().contains("app"));
    }

    #[test]
    fn now_rfc3339_ends_in_z() {
        let s = now_rfc3339();
        assert!(s.ends_with('Z'), "expected UTC timestamp, got {s}");
        assert_eq!(s.len(), 20, "expected second precision, got {s}");
    }
}
