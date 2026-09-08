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
    /// Processes using the most CPU and the most memory, in one response.
    ///
    /// cpu_pct is per-process and exceeds 100 on multi-core work, like `ps`.
    Top {
        /// How many rows per dimension.
        #[arg(short = 'n', long = "count", default_value_t = 5)]
        n: usize,
        /// Accepted for compatibility; JSON is already the default.
        #[arg(long)]
        json: bool,
        /// Print an aligned table instead of JSON.
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

use vitals_core::procs::{collect, rank, to_rows, CPU_WINDOW_MS};
use vitals_core::schema::TopReport;

pub fn collect_top(n: usize) -> Result<TopReport, String> {
    let rows = collect();
    if rows.is_empty() {
        return Err("process enumeration returned nothing".to_string());
    }
    let (by_cpu, by_mem) = rank(rows, n);
    Ok(TopReport {
        schema_version: vitals_core::schema::SCHEMA_VERSION,
        sampled_at: now_rfc3339(),
        sample_ms: CPU_WINDOW_MS,
        by_cpu: to_rows(&by_cpu),
        by_mem: to_rows(&by_mem),
    })
}

pub fn run_top(n: usize, human: bool) -> Result<(), String> {
    let report = collect_top(n)?;
    if human {
        // Two rankings, labelled. Concatenating them under one header lets
        // the same pid appear twice with nothing saying which column put it
        // there, which reads as a duplicate rather than as two answers.
        for (title, rows) in [("BY CPU", &report.by_cpu), ("BY MEM", &report.by_mem)] {
            println!("{title}");
            println!("{:>7}  {:>7}  {:>8}  {}", "PID", "CPU%", "MEM MB", "NAME");
            for r in rows.iter() {
                println!("{:>7}  {:>7.1}  {:>8}  {}", r.pid, r.cpu_pct, r.mem_mb, r.name);
            }
        }
    } else {
        println!("{}", serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?);
    }
    Ok(())
}
