//! The verdict verb.
//!
//! `evaluate` is pure: every input arrives in `PressureInputs`, so the whole
//! rule table is testable without touching hardware or the clock. The rules
//! are a closed set — adding one means adding a row here and a test below,
//! never a special case at a call site.

use crate::history::Observation;
use crate::procs::ProcSample;
use crate::schema::{
    PressureLevel, Reason, Severity, Suspect, ThermalState, Verdict, SCHEMA_VERSION,
};

const SWAP_GROWTH_MB: u64 = 256;
const SWAP_GROWTH_FAST_MB: u64 = 1024;
const MEM_FULL_RATIO: f64 = 0.90;
const LOAD_SATURATED: f64 = 2.0;
const LOAD_SATURATED_HARD: f64 = 4.0;

pub struct PressureInputs {
    pub now: Observation,
    /// `None` when no usable baseline exists; swap rules are then skipped.
    pub prev: Option<Observation>,
    pub mem_pressure: PressureLevel,
    pub thermal: ThermalState,
    pub load1: f64,
    pub ncpu: usize,
    pub top_cpu: Option<ProcSample>,
    pub top_mem: Option<ProcSample>,
    pub sampled_at: String,
}

fn reason(code: &str, severity: Severity, detail: String) -> Reason {
    Reason { code: code.to_string(), severity, detail }
}

/// "a", "a and b", "a, b and c".
pub fn join_with_and(parts: &[String]) -> String {
    match parts {
        [] => String::new(),
        [a] => a.clone(),
        [rest @ .., last] => format!("{} and {}", rest.join(", "), last),
    }
}

pub fn evaluate(i: PressureInputs) -> Verdict {
    let mut reasons: Vec<Reason> = Vec::new();

    match i.mem_pressure {
        PressureLevel::Warning => reasons.push(reason(
            "mem_pressure_warning",
            Severity::Warning,
            "kernel memory pressure level: warning".to_string(),
        )),
        PressureLevel::Critical => reasons.push(reason(
            "mem_pressure_critical",
            Severity::Critical,
            "kernel memory pressure level: critical".to_string(),
        )),
        PressureLevel::Normal | PressureLevel::Unknown => {}
    }

    let history = i.prev.is_some();
    let mut swapping = false;
    if let Some(prev) = i.prev {
        let grew = i.now.swap_used_mb.saturating_sub(prev.swap_used_mb);
        let elapsed = i.now.unix_s.saturating_sub(prev.unix_s);
        if grew >= SWAP_GROWTH_FAST_MB {
            swapping = true;
            reasons.push(reason(
                "swap_growth_fast",
                Severity::Critical,
                format!("swap grew {grew} MB in the last {elapsed}s"),
            ));
        } else if grew >= SWAP_GROWTH_MB {
            swapping = true;
            reasons.push(reason(
                "swap_growth",
                Severity::Warning,
                format!("swap grew {grew} MB in the last {elapsed}s"),
            ));
        }
    }

    if i.now.mem_total_mb > 0 {
        let used_ratio = i.now.mem_used_mb as f64 / i.now.mem_total_mb as f64;
        if used_ratio >= MEM_FULL_RATIO {
            reasons.push(reason(
                "mem_full",
                Severity::Warning,
                format!("{:.0}% of physical memory is in use", used_ratio * 100.0),
            ));
        }
    }

    match i.thermal {
        ThermalState::Serious => reasons.push(reason(
            "thermal_serious",
            Severity::Warning,
            "thermal state: serious".to_string(),
        )),
        ThermalState::Critical => reasons.push(reason(
            "thermal_critical",
            Severity::Critical,
            "thermal state: critical".to_string(),
        )),
        _ => {}
    }

    let load_ratio = if i.ncpu > 0 { i.load1 / i.ncpu as f64 } else { 0.0 };
    if load_ratio >= LOAD_SATURATED_HARD {
        reasons.push(reason(
            "cpu_saturated_hard",
            Severity::Critical,
            format!("1-minute load is {load_ratio:.1}x the core count"),
        ));
    } else if load_ratio >= LOAD_SATURATED {
        reasons.push(reason(
            "cpu_saturated",
            Severity::Warning,
            format!("1-minute load is {load_ratio:.1}x the core count"),
        ));
    }

    let state = reasons
        .iter()
        .map(|r| r.severity)
        .max()
        .unwrap_or(Severity::Nominal);

    let mem_fired = reasons
        .iter()
        .any(|r| r.code.starts_with("mem") || r.code.starts_with("swap"));
    let cpu_fired = reasons
        .iter()
        .any(|r| r.code.starts_with("cpu") || r.code.starts_with("thermal"));

    let mut suspects: Vec<Suspect> = Vec::new();
    if mem_fired {
        if let Some(p) = &i.top_mem {
            suspects.push(Suspect {
                pid: p.pid,
                name: p.name.clone(),
                app: p.app.clone(),
                why: format!("largest resident set ({} MB)", p.mem_mb),
            });
        }
    }
    if cpu_fired {
        if let Some(p) = &i.top_cpu {
            if !suspects.iter().any(|s| s.pid == p.pid) {
                suspects.push(Suspect {
                    pid: p.pid,
                    name: p.name.clone(),
                    app: p.app.clone(),
                    why: format!("highest CPU ({:.0}%)", p.cpu_pct),
                });
            }
        }
    }

    let summary = summarize(&reasons, &suspects, swapping, load_ratio, mem_fired, cpu_fired);

    Verdict {
        schema_version: SCHEMA_VERSION,
        sampled_at: i.sampled_at,
        state,
        history,
        reasons,
        suspects,
        summary,
    }
}

fn summarize(
    reasons: &[Reason],
    suspects: &[Suspect],
    swapping: bool,
    load_ratio: f64,
    mem_fired: bool,
    cpu_fired: bool,
) -> String {
    if reasons.is_empty() {
        return "System is healthy.".to_string();
    }

    let mut clauses: Vec<String> = Vec::new();
    if mem_fired {
        clauses.push("memory is under pressure".to_string());
    }
    if swapping {
        clauses.push("the system is swapping".to_string());
    }
    if reasons.iter().any(|r| r.code.starts_with("cpu")) {
        clauses.push(format!("CPU load is {load_ratio:.1}x the core count"));
    }
    if reasons.iter().any(|r| r.code.starts_with("thermal")) {
        clauses.push("the SoC is thermally throttled".to_string());
    }

    let mut s = join_with_and(&clauses);
    if let Some(first) = s.get_mut(0..1) {
        first.make_ascii_uppercase();
    }
    s.push('.');

    let cause = suspects
        .first()
        .map(|p| p.app.clone().unwrap_or_else(|| p.name.clone()));
    match (cause, cpu_fired && mem_fired, suspects.get(1)) {
        (Some(c), true, Some(second)) => {
            let busiest = second.app.clone().unwrap_or_else(|| second.name.clone());
            s.push_str(&format!(
                " {c} is the likely cause, and {busiest} is the busiest process."
            ));
        }
        (Some(c), _, _) => s.push_str(&format!(" {c} is the likely cause.")),
        (None, _, _) => {}
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::Observation;
    use crate::procs::ProcSample;

    fn obs(unix_s: u64, mem_used: u64, swap: u64) -> Observation {
        Observation { schema_version: 1, unix_s, mem_used_mb: mem_used, mem_total_mb: 36864, swap_used_mb: swap }
    }

    fn base() -> PressureInputs {
        PressureInputs {
            now: obs(1_000_060, 10_000, 100),
            prev: Some(obs(1_000_000, 10_000, 100)),
            mem_pressure: PressureLevel::Normal,
            thermal: ThermalState::Nominal,
            load1: 1.0,
            ncpu: 12,
            top_cpu: None,
            top_mem: None,
            sampled_at: "2026-09-07T22:04:05Z".to_string(),
        }
    }

    fn proc(pid: u32, name: &str, app: Option<&str>, cpu: f32, mem: u64) -> ProcSample {
        ProcSample { pid, name: name.into(), app: app.map(String::from),
                     cpu_pct: cpu, mem_mb: mem, threads: 8 }
    }

    fn codes(v: &Verdict) -> Vec<String> {
        v.reasons.iter().map(|r| r.code.clone()).collect()
    }

    #[test]
    fn a_quiet_machine_is_nominal() {
        let v = evaluate(base());
        assert_eq!(v.state, Severity::Nominal);
        assert!(v.reasons.is_empty());
        assert!(v.suspects.is_empty());
        assert_eq!(v.summary, "System is healthy.");
        assert!(v.history);
        assert_eq!(v.schema_version, 1);
    }

    #[test]
    fn kernel_pressure_maps_straight_through() {
        let mut i = base();
        i.mem_pressure = PressureLevel::Warning;
        assert_eq!(evaluate(i).state, Severity::Warning);

        let mut i = base();
        i.mem_pressure = PressureLevel::Critical;
        let v = evaluate(i);
        assert_eq!(v.state, Severity::Critical);
        assert_eq!(codes(&v), vec!["mem_pressure_critical"]);
    }

    #[test]
    fn swap_growth_fires_at_the_documented_threshold() {
        let mut i = base();
        i.now = obs(1_000_060, 10_000, 100 + 255);
        assert!(evaluate(i).reasons.is_empty(), "255 MB is below the 256 MB threshold");

        let mut i = base();
        i.now = obs(1_000_063, 10_000, 100 + 412);
        let v = evaluate(i);
        assert_eq!(codes(&v), vec!["swap_growth"]);
        assert_eq!(v.reasons[0].detail, "swap grew 412 MB in the last 63s");
    }

    #[test]
    fn fast_swap_growth_suppresses_the_warning_tier() {
        let mut i = base();
        i.now = obs(1_000_060, 10_000, 100 + 2048);
        let v = evaluate(i);
        assert_eq!(codes(&v), vec!["swap_growth_fast"]);
        assert_eq!(v.state, Severity::Critical);
    }

    #[test]
    fn swap_shrinking_is_not_a_reason() {
        let mut i = base();
        i.prev = Some(obs(1_000_000, 10_000, 5000));
        i.now = obs(1_000_060, 10_000, 100);
        assert!(evaluate(i).reasons.is_empty());
    }

    #[test]
    fn without_history_swap_rules_are_skipped_but_others_still_run() {
        let mut i = base();
        i.prev = None;
        i.mem_pressure = PressureLevel::Warning;
        let v = evaluate(i);
        assert!(!v.history);
        assert_eq!(codes(&v), vec!["mem_pressure_warning"]);
    }

    #[test]
    fn mem_full_fires_at_ninety_percent() {
        let mut i = base();
        i.now = obs(1_000_060, 33_178, 100); // 36864 * 0.90 = 33177.6
        assert_eq!(codes(&evaluate(i)), vec!["mem_full"]);

        let mut i = base();
        i.now = obs(1_000_060, 33_000, 100);
        assert!(evaluate(i).reasons.is_empty());
    }

    #[test]
    fn load_saturation_has_two_tiers() {
        let mut i = base();
        i.load1 = 24.0; // 2.0x of 12 cores
        assert_eq!(codes(&evaluate(i)), vec!["cpu_saturated"]);

        let mut i = base();
        i.load1 = 48.0; // 4.0x
        let v = evaluate(i);
        assert_eq!(codes(&v), vec!["cpu_saturated_hard"]);
        assert_eq!(v.state, Severity::Critical);
    }

    #[test]
    fn thermal_states_map_to_reasons() {
        let mut i = base();
        i.thermal = ThermalState::Serious;
        assert_eq!(codes(&evaluate(i)), vec!["thermal_serious"]);

        let mut i = base();
        i.thermal = ThermalState::Fair;
        assert!(evaluate(i).reasons.is_empty(), "fair is not worth reporting");
    }

    #[test]
    fn memory_reasons_blame_the_largest_process_and_cpu_reasons_the_busiest() {
        let mut i = base();
        i.mem_pressure = PressureLevel::Warning;
        i.top_mem = Some(proc(4412, "Google Chrome Helper (Renderer)", Some("Google Chrome"), 12.0, 4820));
        i.top_cpu = Some(proc(77, "cc1plus", None, 380.0, 900));
        let v = evaluate(i);
        assert_eq!(v.suspects.len(), 1);
        assert_eq!(v.suspects[0].pid, 4412);
        assert_eq!(v.suspects[0].why, "largest resident set (4820 MB)");
        assert_eq!(v.summary,
                   "Memory is under pressure. Google Chrome is the likely cause.");
    }

    #[test]
    fn combined_reasons_produce_one_readable_sentence() {
        let mut i = base();
        i.mem_pressure = PressureLevel::Warning;
        i.now = obs(1_000_060, 10_000, 100 + 412);
        i.load1 = 24.0;
        i.top_mem = Some(proc(4412, "Chrome", Some("Google Chrome"), 12.0, 4820));
        i.top_cpu = Some(proc(77, "cc1plus", None, 380.0, 900));
        let v = evaluate(i);
        assert_eq!(v.state, Severity::Warning);
        assert_eq!(
            v.summary,
            "Memory is under pressure, the system is swapping and CPU load is 2.0x the core count. \
             Google Chrome is the likely cause, and cc1plus is the busiest process."
        );
        assert_eq!(v.suspects.len(), 2);
    }

    #[test]
    fn join_with_and_reads_like_english() {
        assert_eq!(join_with_and(&["a".into()]), "a");
        assert_eq!(join_with_and(&["a".into(), "b".into()]), "a and b");
        assert_eq!(join_with_and(&["a".into(), "b".into(), "c".into()]), "a, b and c");
    }
}
