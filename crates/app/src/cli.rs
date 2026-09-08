use clap::{Parser, Subcommand};
use vitals_core::host::{load_avg, uptime_s};
use vitals_core::sample::sample_once;
use vitals_core::schema::{build_snapshot, now_rfc3339, Snapshot, SnapshotInputs};
use vitals_core::sysctl::mem_pressure_level;
use vitals_core::thermal::thermal_state;

/// Default sampling window. Short enough to feel instant to an agent, long
/// enough for stable IOReport deltas.
pub const DEFAULT_INTERVAL_MS: u32 = 200;

#[derive(Parser, Debug)]
#[command(name = "vitals", version, about = "Apple Silicon system monitor")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// One SoC sample: CPU, GPU, memory, power, temperature.
    Snapshot {
        /// Sampling window in milliseconds.
        #[arg(long, default_value_t = DEFAULT_INTERVAL_MS)]
        interval: u32,
        /// Accepted for compatibility; JSON is already the default.
        #[arg(long)]
        json: bool,
        /// Print a compact human-readable line instead of JSON.
        #[arg(long)]
        human: bool,
    },
}

/// Collect a snapshot with every field populated.
pub fn collect_snapshot(interval_ms: u32) -> Result<Snapshot, String> {
    let sample = sample_once(interval_ms)?;
    Ok(build_snapshot(SnapshotInputs {
        metrics: &sample.metrics,
        host: sample.host.clone(),
        sample_ms: sample.sample_ms,
        sampled_at: now_rfc3339(),
        mem_pressure: mem_pressure_level(),
        thermal_state: thermal_state(),
        load_avg: load_avg(),
        uptime_s: uptime_s(),
    }))
}

pub fn run_snapshot(interval_ms: u32, human: bool) -> Result<(), String> {
    let snap = collect_snapshot(interval_ms)?;
    if human {
        println!(
            "cpu {:.1}%  gpu {:.1}%  mem {}/{} MB  swap {} MB  {:.2} W  {:.1}°C",
            snap.cpu_pct, snap.gpu_pct, snap.mem_used_mb, snap.mem_total_mb,
            snap.swap_used_mb, snap.power_total_w, snap.temp_cpu_c
        );
    } else {
        println!("{}", serde_json::to_string_pretty(&snap).map_err(|e| e.to_string())?);
    }
    Ok(())
}
