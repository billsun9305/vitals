//! Per-process metrics — entirely outside `macmon`'s scope.
//!
//! This is the most expensive code in the crate (it enumerates every process
//! twice, `MINIMUM_CPU_UPDATE_INTERVAL` apart) and must therefore never run
//! on the tray's tick. Only `top` and `pressure` call it.

use crate::schema::ProcRow;
use libproc::proc_pid::pidinfo;
use libproc::task_info::TaskInfo;
use std::collections::HashMap;
use sysinfo::{
    ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind, MINIMUM_CPU_UPDATE_INTERVAL,
};

/// How far up the parent chain `resolve_app` will walk.
const MAX_PARENT_DEPTH: usize = 8;

/// The window per-process CPU percentages are measured over — exactly the
/// interval `collect` sleeps between its two refreshes.
///
/// Derived from `sysinfo` rather than copied, so a change upstream cannot
/// leave callers reporting a window that is no longer the real one. A
/// hand-written literal here would still compile and still pass every test
/// while quietly lying about how the numbers were measured.
pub const CPU_WINDOW_MS: u32 = MINIMUM_CPU_UPDATE_INTERVAL.as_millis() as u32;

/// Raw per-process facts, before app resolution.
#[derive(Debug, Clone)]
pub struct RawProc {
    pub pid: u32,
    pub parent: Option<u32>,
    pub name: String,
    pub exe: Option<String>,
    pub cpu_pct: f32,
    pub mem_bytes: u64,
}

/// A finished row, before it is narrowed to the wire type.
#[derive(Debug, Clone)]
pub struct ProcSample {
    pub pid: u32,
    pub name: String,
    pub app: Option<String>,
    pub cpu_pct: f32,
    pub mem_mb: u64,
    pub threads: u32,
}

/// Extract the bundle display name from an executable path, or `None` when
/// the path is not inside an app bundle's `Contents/MacOS` directory.
///
/// A helper process can live inside a nested `.app` (e.g. a renderer helper
/// packaged under the browser's `Frameworks` directory); the outermost
/// bundle is the one a human recognizes, so this takes the *first* `.app/`
/// boundary in the path, after confirming the path is genuinely inside some
/// bundle's `Contents/MacOS` (which a tool path like `Xcode.app/Contents/
/// Developer/usr/bin/clang` is not).
pub fn app_name(exe: &str) -> Option<String> {
    if !exe.contains("/Contents/MacOS/") {
        return None;
    }
    let idx = exe.find(".app/")?;
    let name = exe[..idx].rsplit('/').next()?;
    if name.is_empty() {
        None
    } else {
        Some(name.to_string())
    }
}

/// Walk from `start` up the parent chain, returning the first bundle name found.
pub fn resolve_app(start: u32, by_pid: &HashMap<u32, RawProc>) -> Option<String> {
    let mut pid = start;
    for _ in 0..MAX_PARENT_DEPTH {
        let p = by_pid.get(&pid)?;
        if let Some(exe) = p.exe.as_deref() {
            if let Some(name) = app_name(exe) {
                return Some(name);
            }
        }
        pid = p.parent?;
    }
    None
}

fn thread_count(pid: u32) -> u32 {
    // `sysinfo::Process::thread_kind()` is Linux-only, so go to the source.
    pidinfo::<TaskInfo>(pid as i32, 0)
        .map(|t| t.pti_threadnum.max(0) as u32)
        .unwrap_or(0)
}

/// Enumerate every visible process, measuring CPU across one refresh interval.
pub fn collect() -> Vec<ProcSample> {
    let kind = ProcessRefreshKind::nothing()
        .with_cpu()
        .with_memory()
        .with_exe(UpdateKind::OnlyIfNotSet);

    let mut sys = System::new();
    sys.refresh_processes_specifics(ProcessesToUpdate::All, true, kind);
    std::thread::sleep(MINIMUM_CPU_UPDATE_INTERVAL);
    sys.refresh_processes_specifics(ProcessesToUpdate::All, true, kind);

    let by_pid: HashMap<u32, RawProc> = sys
        .processes()
        .iter()
        .map(|(pid, p)| {
            let pid = pid.as_u32();
            (
                pid,
                RawProc {
                    pid,
                    parent: p.parent().map(|p| p.as_u32()),
                    name: p.name().to_string_lossy().into_owned(),
                    exe: p.exe().map(|e| e.to_string_lossy().into_owned()),
                    cpu_pct: p.cpu_usage(),
                    mem_bytes: p.memory(),
                },
            )
        })
        .collect();

    by_pid
        .values()
        .filter(|p| p.pid != 0)
        .map(|p| ProcSample {
            pid: p.pid,
            name: p.name.clone(),
            app: resolve_app(p.pid, &by_pid),
            cpu_pct: (p.cpu_pct * 10.0).round() / 10.0,
            mem_mb: p.mem_bytes / (1024 * 1024),
            threads: thread_count(p.pid),
        })
        .collect()
}

/// Top `n` by CPU and top `n` by memory, each sorted descending with pid as
/// the tie-break so repeated runs are byte-identical on an idle machine.
pub fn rank(rows: Vec<ProcSample>, n: usize) -> (Vec<ProcSample>, Vec<ProcSample>) {
    let mut by_cpu = rows.clone();
    by_cpu.sort_by(|a, b| {
        b.cpu_pct
            .total_cmp(&a.cpu_pct)
            .then_with(|| a.pid.cmp(&b.pid))
    });
    by_cpu.truncate(n);

    let mut by_mem = rows;
    by_mem.sort_by(|a, b| b.mem_mb.cmp(&a.mem_mb).then_with(|| a.pid.cmp(&b.pid)));
    by_mem.truncate(n);

    (by_cpu, by_mem)
}

pub fn to_rows(samples: &[ProcSample]) -> Vec<ProcRow> {
    samples
        .iter()
        .map(|p| ProcRow {
            pid: p.pid,
            name: p.name.clone(),
            app: p.app.clone(),
            cpu_pct: p.cpu_pct,
            mem_mb: p.mem_mb,
            threads: p.threads,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn app_name_extracts_the_bundle() {
        assert_eq!(
            app_name("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"),
            Some("Google Chrome".to_string())
        );
    }

    #[test]
    fn app_name_prefers_the_outermost_bundle() {
        // Helper processes live inside a nested .app; the outer bundle is the
        // one a human recognizes, and `find` returns the first match.
        let exe = "/Applications/Google Chrome.app/Contents/Frameworks/\
                   Google Chrome Framework.framework/Versions/1/Helpers/\
                   Google Chrome Helper.app/Contents/MacOS/Google Chrome Helper";
        assert_eq!(app_name(exe), Some("Google Chrome".to_string()));
    }

    #[test]
    fn app_name_rejects_non_bundle_paths() {
        assert_eq!(app_name("/usr/bin/ssh"), None);
        assert_eq!(
            app_name("/Applications/Xcode.app/Contents/Developer/usr/bin/clang"),
            None
        );
        assert_eq!(app_name(""), None);
    }

    fn raw(pid: u32, parent: Option<u32>, exe: &str) -> RawProc {
        RawProc {
            pid,
            parent,
            name: format!("p{pid}"),
            exe: Some(exe.into()),
            cpu_pct: 0.0,
            mem_bytes: 0,
        }
    }

    #[test]
    fn resolve_app_walks_up_to_the_owning_bundle() {
        let mut by_pid = HashMap::new();
        by_pid.insert(1, raw(1, None, "/sbin/launchd"));
        by_pid.insert(
            10,
            raw(
                10,
                Some(1),
                "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
            ),
        );
        by_pid.insert(20, raw(20, Some(10), "/usr/lib/helper"));
        assert_eq!(resolve_app(20, &by_pid), Some("Google Chrome".to_string()));
        assert_eq!(resolve_app(1, &by_pid), None);
    }

    #[test]
    fn resolve_app_survives_a_parent_cycle() {
        let mut by_pid = HashMap::new();
        by_pid.insert(5, raw(5, Some(6), "/usr/lib/a"));
        by_pid.insert(6, raw(6, Some(5), "/usr/lib/b"));
        assert_eq!(resolve_app(5, &by_pid), None); // terminates, does not hang
    }

    fn sample(pid: u32, cpu: f32, mem_mb: u64) -> ProcSample {
        ProcSample {
            pid,
            name: format!("p{pid}"),
            app: None,
            cpu_pct: cpu,
            mem_mb,
            threads: 1,
        }
    }

    #[test]
    fn rank_returns_top_n_by_each_dimension() {
        let rows = vec![
            sample(1, 10.0, 500),
            sample(2, 300.0, 10),
            sample(3, 5.0, 4000),
        ];
        let (by_cpu, by_mem) = rank(rows, 2);
        assert_eq!(by_cpu.iter().map(|p| p.pid).collect::<Vec<_>>(), vec![2, 1]);
        assert_eq!(by_mem.iter().map(|p| p.pid).collect::<Vec<_>>(), vec![3, 1]);
    }

    #[test]
    fn rank_breaks_ties_by_pid_for_determinism() {
        let rows = vec![sample(9, 1.0, 1), sample(2, 1.0, 1), sample(5, 1.0, 1)];
        let (by_cpu, _) = rank(rows, 3);
        assert_eq!(
            by_cpu.iter().map(|p| p.pid).collect::<Vec<_>>(),
            vec![2, 5, 9]
        );
    }

    #[test]
    fn collect_sees_this_test_process() {
        let me = std::process::id();
        let rows = collect();
        let mine = rows
            .iter()
            .find(|p| p.pid == me)
            .expect("own process missing from table");
        assert!(mine.threads >= 1, "thread count should come from libproc");
        assert!(mine.mem_mb >= 1);
    }

    #[test]
    fn collect_measures_nonzero_cpu_somewhere() {
        // `collect()` must refresh twice, `MINIMUM_CPU_UPDATE_INTERVAL` apart,
        // because sysinfo derives CPU percent from the delta between two
        // refreshes — a single refresh silently reports 0% for every
        // process. Don't assert on this test process's own cpu_pct (an idle
        // process legitimately reads 0.0, which would be flaky); instead
        // assert across the whole table, since some process on a running Mac
        // is always burning CPU.
        let rows = collect();
        assert!(
            rows.iter().any(|p| p.cpu_pct > 0.0),
            "expected at least one process with nonzero CPU; double-refresh may be broken"
        );
    }
}
