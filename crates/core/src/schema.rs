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

#[cfg(test)]
mod tests {
    use super::*;

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
