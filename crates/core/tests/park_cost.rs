use std::sync::Arc;
use std::time::{Duration, Instant};
use vitals_core::cadence::Cadence;
use vitals_core::sample::spawn_sampler;

fn self_cpu_secs() -> f64 {
    // getrusage(RUSAGE_SELF) via libc-free FFI
    #[repr(C)]
    #[derive(Default)]
    struct Timeval { sec: i64, usec: i32, _pad: i32 }
    #[repr(C)]
    #[derive(Default)]
    struct Rusage { utime: Timeval, stime: Timeval, rest: [i64; 14] }
    extern "C" { fn getrusage(who: i32, usage: *mut Rusage) -> i32; }
    let mut r = Rusage::default();
    unsafe { getrusage(0, &mut r) };
    r.utime.sec as f64 + r.utime.usec as f64 / 1e6
        + r.stime.sec as f64 + r.stime.usec as f64 / 1e6
}

/// Ignored by default: it sleeps ~7s. Run with
/// `cargo test -p vitals-core --test park_cost -- --ignored --nocapture`.
///
/// This is the mechanism-level guard behind the plan's idle budget. If
/// `wait_while_parked` ever degrades from a condvar block into a poll, the
/// tray's whole power story collapses silently — nothing else in the suite
/// would notice, because a spinning worker still delivers correct samples.
#[test]
#[ignore]
fn parked_worker_costs_nothing() {
    let cadence = Arc::new(Cadence::new(200));
    let handle = spawn_sampler(Arc::clone(&cadence));
    // let it actually start sampling, then measure the RUNNING cost
    std::thread::sleep(Duration::from_millis(600));
    let t0 = Instant::now(); let c0 = self_cpu_secs();
    std::thread::sleep(Duration::from_secs(3));
    let running = (self_cpu_secs() - c0) / t0.elapsed().as_secs_f64();

    // now park and measure again, excluding the in-flight sample
    cadence.park();
    std::thread::sleep(Duration::from_millis(600));
    let t1 = Instant::now(); let c1 = self_cpu_secs();
    std::thread::sleep(Duration::from_secs(3));
    let parked = (self_cpu_secs() - c1) / t1.elapsed().as_secs_f64();

    println!("RUNNING cpu = {:.4}%   PARKED cpu = {:.4}%", running * 100.0, parked * 100.0);
    assert!(parked < 0.001, "parked worker burned {:.4}% CPU — it is spinning", parked * 100.0);
    assert!(parked < running / 10.0, "parking did not meaningfully reduce cost");
    cadence.unpark();
    drop(handle);
}
