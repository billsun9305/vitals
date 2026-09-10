use clap::{Parser, Subcommand};
use vitals_core::host::{load_avg, uptime_s};
use vitals_core::sample::sample_once;
use vitals_core::schema::{build_snapshot, now_rfc3339, Snapshot, SnapshotInputs};
use vitals_core::sysctl::mem_pressure_level;
use vitals_core::thermal::thermal_state;

/// Default sampling window. Short enough to feel instant to an agent, long
/// enough for stable IOReport deltas.
pub const DEFAULT_INTERVAL_MS: u32 = 200;

/// Floor on any requested sampling window.
///
/// `--interval 0` used to be accepted and produce a straight-faced reading
/// with `"sample_ms": 0` and junk in it (a 50% CPU figure on an idle
/// machine), because IOReport deltas over a zero-length window are
/// meaningless. `watch` already clamped 0 to one second; the one-shot verbs
/// silently did not, so the same input meant two different things. Clamping
/// rather than rejecting matches `watch`, and 20ms is low enough that anyone
/// deliberately asking for a tight window still gets one.
pub const MIN_INTERVAL_MS: u32 = 20;

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
    /// A semantic health verdict: state, reasons, likely suspects, one sentence.
    Pressure {
        /// Sampling window in milliseconds.
        #[arg(long, default_value_t = DEFAULT_INTERVAL_MS)]
        interval: u32,
        /// Accepted for compatibility; JSON is already the default.
        #[arg(long)]
        json: bool,
        /// Print only the summary sentence.
        #[arg(long)]
        human: bool,
        /// Exit 0 nominal, 3 warning, 4 critical. A real error is still 1.
        #[arg(long)]
        exit_code: bool,
    },
    /// Stream snapshots as newline-delimited JSON until interrupted.
    Watch {
        /// Seconds between samples; also the sampling window.
        #[arg(short = 'i', long = "interval", default_value_t = 2)]
        interval_s: u64,
        /// Stop after this many samples. 0 means run forever.
        #[arg(short = 'n', long = "count", default_value_t = 0)]
        count: u64,
    },
    /// Serve the JSON API and the dashboard on localhost.
    Serve {
        #[arg(long, default_value_t = crate::serve::DEFAULT_PORT)]
        port: u16,
        /// Do not open a browser window.
        #[arg(long)]
        no_open: bool,
    },
    /// Start the server if needed and open the dashboard.
    Dashboard {
        #[arg(long, default_value_t = crate::serve::DEFAULT_PORT)]
        port: u16,
    },
    /// Host the dashboard in a native window.
    ///
    /// Spawned by the menu bar app, which owns the server the window talks
    /// to; not meant to be run by hand, and hidden from `--help` for that
    /// reason.
    #[command(hide = true)]
    Window {
        /// The dashboard URL to load, e.g. http://127.0.0.1:9876/.
        #[arg(long)]
        url: String,
        /// Pid of the tray that spawned this window; the window exits when
        /// that process does, and "Quit Vitals" from the window quits it.
        #[arg(long)]
        parent: u32,
    },
}

/// Collect a snapshot with every field populated.
pub fn collect_snapshot(interval_ms: u32) -> Result<Snapshot, String> {
    let sample = sample_once(interval_ms.max(MIN_INTERVAL_MS))?;
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
            snap.cpu_pct,
            snap.gpu_pct,
            snap.mem_used_mb,
            snap.mem_total_mb,
            snap.swap_used_mb,
            snap.power_total_w,
            snap.temp_cpu_c
        );
    } else {
        println!(
            "{}",
            serde_json::to_string_pretty(&snap).map_err(|e| e.to_string())?
        );
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
            // Column widths here must match the row format on the next line.
            println!("    PID     CPU%    MEM MB  NAME");
            for r in rows.iter() {
                println!(
                    "{:>7}  {:>7.1}  {:>8}  {}",
                    r.pid, r.cpu_pct, r.mem_mb, r.name
                );
            }
        }
    } else {
        println!(
            "{}",
            serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?
        );
    }
    Ok(())
}

use vitals_core::history::{self, Observation};
use vitals_core::pressure::{evaluate, PressureInputs};
use vitals_core::schema::{Severity, Verdict};

pub fn exit_code_for(state: Severity) -> i32 {
    match state {
        Severity::Nominal => 0,
        Severity::Warning => 3,
        Severity::Critical => 4,
    }
}

/// Collect a pressure verdict: one SoC sample and one process pass.
///
/// `macmon::Sampler` is `!Send` so it must stay on this thread; `sysinfo::
/// System` is `Send`, so the process pass runs on a spawned thread instead.
/// The two ~`interval_ms` windows then overlap rather than running back to
/// back, which is one window of latency instead of two.
pub fn collect_pressure(interval_ms: u32) -> Result<Verdict, String> {
    collect_pressure_inner(interval_ms, Baseline::Store)
}

/// Whether this caller is allowed to overwrite the stored trend baseline.
///
/// Only a one-shot invocation should: the baseline exists so that the NEXT
/// question can be answered with a delta. A face that asks continuously —
/// the dashboard polls `/api/pressure` every 5 seconds — would reset it on
/// every poll, leaving every reader a 5-second-old baseline. The swap rules
/// need +256 MB (and +1024 MB for the fast one) BETWEEN observations, so a
/// single open browser tab would silently disable swap-trend detection for
/// everyone, the terminal included.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Baseline {
    Store,
    ReadOnly,
}

pub fn collect_pressure_inner(interval_ms: u32, baseline: Baseline) -> Result<Verdict, String> {
    let procs_thread = std::thread::spawn(collect);
    let snap = collect_snapshot(interval_ms)?;
    finish_pressure(snap, procs_thread, baseline)
}

/// Evaluate a verdict from a snapshot somebody else already took.
///
/// `vitals serve` keeps a background sampler running and caches its latest
/// `Snapshot`. Without this entry point `/api/pressure` went through
/// `collect_snapshot`, which builds a whole new `macmon::Sampler` — roughly
/// half a second of setup — and samples the SoC *concurrently with the
/// server's own sampler*. Two samplers reading IOReport at once is the exact
/// thing the cadence design exists to prevent, and it happened on every
/// dashboard poll.
pub fn pressure_from_snapshot(snap: Snapshot, baseline: Baseline) -> Result<Verdict, String> {
    let procs_thread = std::thread::spawn(collect);
    finish_pressure(snap, procs_thread, baseline)
}

fn finish_pressure(
    snap: Snapshot,
    procs_thread: std::thread::JoinHandle<Vec<vitals_core::procs::ProcSample>>,
    baseline: Baseline,
) -> Result<Verdict, String> {
    let mut rows = procs_thread
        .join()
        .map_err(|_| "process collector panicked".to_string())?;

    // Never let this process be the answer to "what is slowing my Mac down".
    // Its CPU figure here is an artifact of the measurement: sampling
    // IOReport and walking the process table is the busiest this process
    // ever gets, and it happens inside the very window `sysinfo` measures,
    // so it reliably ranks itself at the top. Measured before this line
    // existed: 8 of 8 consecutive runs reported `vitals (highest CPU 77%)`
    // while the real load was ChatGPT and Chrome, and the summary sentence
    // -- the one docs/agents.md tells agents to quote verbatim -- read
    // "vitals is the likely cause."
    //
    // Filtering here rather than inside `pressure::evaluate` is deliberate:
    // `evaluate` receives only the single top row per dimension, so dropping
    // it there would leave no suspect at all instead of promoting the real
    // one. `top` is left alone on purpose -- it is a raw ranking, and during
    // a `vitals top` run this process genuinely is using that CPU.
    let me = std::process::id();
    rows.retain(|r| r.pid != me);

    let (by_cpu, by_mem) = rank(rows, 1);

    let now_s = history::unix_now_s();
    let observation = Observation {
        schema_version: vitals_core::schema::SCHEMA_VERSION,
        unix_s: now_s,
        mem_used_mb: snap.mem_used_mb,
        mem_total_mb: snap.mem_total_mb,
        swap_used_mb: snap.swap_used_mb,
    };
    let prev = history::load(now_s);

    let verdict = evaluate(PressureInputs {
        now: observation,
        prev,
        mem_pressure: snap.mem_pressure,
        thermal: snap.thermal_state,
        load1: snap.load_avg[0],
        ncpu: snap.host.ncpu,
        top_cpu: by_cpu.into_iter().next(),
        top_mem: by_mem.into_iter().next(),
        sampled_at: snap.sampled_at.clone(),
    });

    // Store after evaluating, so this run's numbers become the next
    // baseline. History is best-effort by design: a write failure must not
    // fail the command, since the verdict computed above is already valid.
    if baseline == Baseline::Store {
        let _ = history::store(&observation);
    }

    Ok(verdict)
}

pub fn run_pressure(interval_ms: u32, human: bool, use_exit_code: bool) -> Result<(), String> {
    let verdict = collect_pressure(interval_ms)?;
    if human {
        println!("{}", verdict.summary);
    } else {
        println!(
            "{}",
            serde_json::to_string_pretty(&verdict).map_err(|e| e.to_string())?
        );
    }
    if use_exit_code {
        std::process::exit(exit_code_for(verdict.state));
    }
    Ok(())
}

use std::io::Write;
use vitals_core::sample::SamplerSession;

/// Stream snapshots as NDJSON until `count` lines have been emitted (or
/// forever, when `count` is 0). One `SamplerSession` is kept alive for the
/// whole run, so `Sampler::new` is paid once rather than per line.
/// `interval_s` of 0 is clamped to 1 second rather than rejected.
///
/// The window passed to `next` is the *full* inter-line interval, unlike
/// core's background worker, which measures over a short window and sleeps
/// out the remainder. The difference is about interruptibility, not
/// correctness: the worker must stay reachable so an open menu can retune
/// it mid-gap, whereas a CLI stream has nothing to stay responsive to.
/// Back-to-back calls make the window and the period the same thing, so
/// the simpler shape is also the correct one here.
///
/// Worth knowing before changing either: `get_metrics` returns as soon as
/// `window_ms` has passed *since the previous call*, counting time already
/// spent blocked elsewhere toward that budget — ~12ms when a 2s sleep
/// preceded a 1s request, against ~1.01s back to back. A worker that
/// sleeps between short windows must therefore time its actual iteration;
/// subtracting a window it never spent makes every period a window short.
/// Core does time it, and its cadence is correct.
pub fn run_watch(interval_s: u64, count: u64) -> Result<(), String> {
    let interval_ms = interval_s.max(1).saturating_mul(1000).min(u32::MAX as u64) as u32;
    let mut session = SamplerSession::new()?;
    let mut emitted = 0u64;
    let stdout = std::io::stdout();

    loop {
        let sample = session.next(interval_ms)?;
        let snap = build_snapshot(SnapshotInputs {
            metrics: &sample.metrics,
            host: sample.host.clone(),
            sample_ms: sample.sample_ms,
            sampled_at: now_rfc3339(),
            mem_pressure: mem_pressure_level(),
            thermal_state: thermal_state(),
            load_avg: load_avg(),
            uptime_s: uptime_s(),
        });

        // Compact, not pretty: NDJSON is one complete object per line, and
        // `to_string_pretty` would spread that object across several lines
        // and break every line-oriented consumer tailing the stream.
        let line = serde_json::to_string(&snap).map_err(|e| e.to_string())?;
        let mut handle = stdout.lock();
        // A closed pipe (`| head -1`) is a normal exit. Anything else —
        // a full disk when stdout is redirected to a file, say — is a real
        // failure, and reporting it as success would leave a caller with
        // silently truncated output and a zero exit status.
        if let Err(e) = writeln!(handle, "{line}").and_then(|()| handle.flush()) {
            return match e.kind() {
                std::io::ErrorKind::BrokenPipe => Ok(()),
                _ => Err(format!("writing to stdout: {e}")),
            };
        }
        drop(handle);

        emitted += 1;
        if count != 0 && emitted >= count {
            return Ok(());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The monitoring process must never be named as the cause of the load
    /// it is reporting.
    ///
    /// This has to be an in-process test: the point is that
    /// `std::process::id()` is the same process `collect()` is measuring.
    /// Driving the binary as a subprocess would compare a pid the test does
    /// not know against a table it cannot see.
    ///
    /// The busy threads matter. Without them this process idles, never ranks
    /// top, and the assertion holds no matter what `collect_pressure` does --
    /// a test that passes for the wrong reason. Saturating every core makes
    /// this process the top CPU consumer for the duration of the sampling
    /// window, which is exactly the condition that produced
    /// "vitals is the likely cause" in 8 of 8 runs before the filter existed.
    #[test]
    fn the_measuring_process_is_never_its_own_suspect() {
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let mut burners = Vec::new();
        for _ in 0..std::thread::available_parallelism().map_or(4, |n| n.get()) {
            let stop = std::sync::Arc::clone(&stop);
            burners.push(std::thread::spawn(move || {
                let mut x = 0u64;
                while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                    x = x.wrapping_mul(6364136223846793005).wrapping_add(1);
                }
                x
            }));
        }

        let verdict = collect_pressure(DEFAULT_INTERVAL_MS).expect("pressure should evaluate");

        stop.store(true, std::sync::atomic::Ordering::Relaxed);
        for b in burners {
            let _ = b.join();
        }

        let me = std::process::id();
        assert!(
            !verdict.suspects.iter().any(|s| s.pid == me),
            "the measuring process (pid {me}) named itself a suspect: {:?}",
            verdict.suspects
        );
    }
}
