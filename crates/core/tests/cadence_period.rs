//! Cadence-period guard. Lives in its own integration test binary, and so
//! its own process, because `park_cost.rs` measures CPU with
//! `getrusage(RUSAGE_SELF)` — which sums every thread in the process. Run in
//! the same binary, this test's sampler threads counted as that test's
//! "parked" cost and pushed it from 0.001% to 6.3%.

use std::sync::Arc;
use std::time::{Duration, Instant};
use vitals_core::cadence::Cadence;

/// Ignored by default: ~16s. Run with
/// `cargo test -p vitals-core --test park_cost -- --ignored --nocapture`.
///
/// `get_metrics` measures from the last sample point rather than blocking
/// afresh, so once a gap has already elapsed it returns in ~12ms. A worker
/// that sleeps `interval - window` on the assumption the window was spent
/// therefore ticks a whole window early — a 5s cadence firing every 4s.
/// Nothing else would notice: the samples are all correct, just too
/// frequent, which quietly spends the idle budget the cadence exists to
/// protect.
#[test]
#[ignore]
fn worker_period_matches_the_requested_interval() {
    use vitals_core::sample::spawn_sampler;

    for interval in [2000u32, 3000] {
        let c = Arc::new(Cadence::new(interval));
        let h = spawn_sampler(Arc::clone(&c));
        let mut stamps = Vec::new();
        let t0 = Instant::now();
        while stamps.len() < 4 && t0.elapsed().as_secs() < 20 {
            if h.try_recv().is_some() {
                stamps.push(Instant::now());
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        c.park();
        assert!(
            stamps.len() >= 4,
            "only {} samples for interval {interval}",
            stamps.len()
        );

        // Skip the first delta: the very first get_metrics has no prior
        // sample point, so it really does block for the whole window and
        // that one iteration is legitimately short.
        let deltas: Vec<u128> = stamps
            .windows(2)
            .skip(1)
            .map(|w| (w[1] - w[0]).as_millis())
            .collect();
        println!("interval={interval}ms -> steady-state deltas {deltas:?}");
        for d in deltas {
            let off = (d as i128 - interval as i128).abs();
            assert!(
                off < 400,
                "interval {interval}ms produced a {d}ms period ({off}ms off)"
            );
        }
    }
}
