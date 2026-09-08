//! Ownership of `macmon::Sampler`.
//!
//! `Sampler` is `!Send` and `!Sync`, so it can never be constructed on one
//! thread and moved to another. Every consumer either calls `sample_once`
//! (build, sample, drop) or builds a `SamplerSession` on the thread that
//! will drive it. `Sample` is deliberately plain data so it *can* cross a
//! channel to the AppKit main thread.

use crate::host::{host_from_soc, ncpu};
use crate::schema::Host;
use macmon::{Metrics, Sampler};

/// One completed sampling window, safe to send between threads.
#[derive(Debug)]
pub struct Sample {
    pub metrics: Metrics,
    pub host: Host,
    pub sample_ms: u32,
}

/// A live `Sampler` plus the static host description, so repeated sampling
/// pays the construction cost once.
pub struct SamplerSession {
    sampler: Sampler,
    host: Host,
}

impl SamplerSession {
    pub fn new() -> Result<Self, String> {
        // `Box<dyn Error>` is not `Send`; stringify before it can escape.
        let sampler = Sampler::new().map_err(|e| format!("Sampler::new failed: {e}"))?;
        let host = host_from_soc(sampler.get_soc_info(), ncpu());
        Ok(Self { sampler, host })
    }

    /// Block for `interval_ms` and return the deltas measured across it.
    pub fn next(&mut self, interval_ms: u32) -> Result<Sample, String> {
        let metrics = self
            .sampler
            .get_metrics(interval_ms)
            .map_err(|e| format!("get_metrics failed: {e}"))?;
        Ok(Sample {
            metrics,
            host: self.host.clone(),
            sample_ms: interval_ms,
        })
    }
}

/// Build a sampler, take one window, drop it. The CLI's whole sampling path.
pub fn sample_once(interval_ms: u32) -> Result<Sample, String> {
    SamplerSession::new()?.next(interval_ms)
}

use crate::cadence::Cadence;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::Arc;

/// The main thread's end of the sampler worker.
pub struct SamplerHandle {
    rx: Receiver<Result<Sample, String>>,
    pub cadence: Arc<Cadence>,
}

impl SamplerHandle {
    /// Non-blocking. Returns the **newest** sample, discarding any older ones
    /// still queued; `None` when nothing new has arrived. The AppKit main
    /// thread must never block on this.
    ///
    /// The channel is unbounded, so returning its front element would make
    /// the tray display data that falls further and further behind whenever
    /// the main thread polls slower than the worker samples — under timer
    /// coalescing, a modal run loop, or just a 1s timer racing the 1s
    /// menu-open cadence. A display wants the latest value, never a backlog,
    /// so drain to it. An error is returned as soon as it is seen rather than
    /// being discarded by a later success.
    pub fn try_recv(&self) -> Option<Result<Sample, String>> {
        let mut newest: Option<Result<Sample, String>> = None;
        loop {
            match self.rx.try_recv() {
                Ok(Err(e)) => return Some(Err(e)),
                Ok(ok) => newest = Some(ok),
                Err(TryRecvError::Empty) => return newest,
                // Deliver buffered data first; the next call reports the stop.
                Err(TryRecvError::Disconnected) => {
                    return newest.or_else(|| Some(Err("sampler thread stopped".to_string())))
                }
            }
        }
    }
}

/// Start the sampler worker.
///
/// The `SamplerSession` is built *inside* the closure: `macmon::Sampler` is
/// `!Send`, so it cannot be constructed here and moved. `Sample` is plain
/// data and crosses the channel freely.
pub fn spawn_sampler(cadence: Arc<Cadence>) -> SamplerHandle {
    let (tx, rx) = mpsc::channel();
    let worker_cadence = Arc::clone(&cadence);

    std::thread::Builder::new()
        .name("vitals-sampler".to_string())
        .spawn(move || {
            let mut session = match SamplerSession::new() {
                Ok(s) => s,
                Err(e) => {
                    let _ = tx.send(Err(e));
                    return;
                }
            };
            loop {
                // Park rather than spin: zero wakeups while the display sleeps.
                worker_cadence.wait_while_parked();
                let interval = worker_cadence.interval_ms();
                // Measure over a short window, then wait out the rest of the
                // interval interruptibly. get_metrics blocks for exactly the
                // window it is given, so sampling for the full interval would
                // pin the worker inside a 30s call on low power and make an
                // open menu wait that long to take effect.
                let window = interval.min(crate::cadence::SAMPLE_WINDOW_MS);
                let tick = std::time::Instant::now();
                match session.next(window) {
                    Ok(sample) => {
                        if tx.send(Ok(sample)).is_err() {
                            return; // main thread went away
                        }
                    }
                    Err(e) => {
                        if tx.send(Err(e)).is_err() {
                            return;
                        }
                    }
                }
                // Sleep the rest of the *measured* iteration, not
                // `interval - window`. get_metrics measures from the last
                // sample point rather than blocking afresh, so once a gap
                // has already elapsed it returns in ~12ms — and subtracting
                // a window that was never spent made every period come out
                // a full window short (a 5s cadence ticking every 4s).
                let spent = tick.elapsed().as_millis().min(u32::MAX as u128) as u32;
                worker_cadence.wait_between_samples(interval.saturating_sub(spent), interval);
            }
        })
        .expect("failed to spawn vitals-sampler thread");

    SamplerHandle { rx, cadence }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sample_once_returns_a_plausible_sample() {
        let s = sample_once(150).expect("sampling failed");
        assert_eq!(s.sample_ms, 150);
        assert!(
            s.metrics.memory.ram_total > 0,
            "ram_total should be non-zero"
        );
        assert!(!s.host.chip.is_empty());
        assert!(
            (0.0..=1.01).contains(&s.metrics.cpu_active_ratio),
            "active ratio out of range: {}",
            s.metrics.cpu_active_ratio
        );
    }

    #[test]
    fn a_session_can_be_reused_without_rebuilding_the_sampler() {
        let mut session = SamplerSession::new().expect("Sampler::new failed");
        let a = session.next(100).expect("first sample failed");
        let b = session.next(100).expect("second sample failed");
        assert!(a.metrics.memory.ram_total > 0 && b.metrics.memory.ram_total > 0);
    }

    #[test]
    fn sample_is_send_so_it_can_cross_a_channel() {
        fn assert_send<T: Send>() {}
        assert_send::<Sample>();
    }

    #[test]
    fn the_requested_interval_actually_gates_the_sample_window() {
        // Sample.sample_ms is a straight echo of the caller's argument, so
        // asserting on it proves only that the number survived the round
        // trip — a sampler that ignored interval_ms entirely and used a
        // fixed window would pass that check. Time it instead.
        use std::time::Instant;
        let mut s = SamplerSession::new().expect("Sampler::new() failed");
        let _ = s.next(100); // first call primes IOReport; don't time it

        let t = Instant::now();
        s.next(500).expect("long sample failed");
        let long = t.elapsed();

        let t = Instant::now();
        s.next(50).expect("short sample failed");
        let short = t.elapsed();

        // Generous bounds: this must not flake on a loaded machine. The
        // load-bearing claim is the lower bound on `long` — the sampler
        // cannot return early, so a hardcoded short window fails here.
        assert!(long.as_millis() >= 450, "500ms sample took only {long:?}");
        assert!(short.as_millis() < 300, "50ms sample took {short:?}");
    }

    /// Helper: poll for a sample, returning how long it took to arrive.
    #[cfg(test)]
    fn wait_for_sample(
        h: &SamplerHandle,
        budget: std::time::Duration,
    ) -> Option<std::time::Duration> {
        let t = std::time::Instant::now();
        while t.elapsed() < budget {
            if h.try_recv().is_some() {
                return Some(t.elapsed());
            }
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
        None
    }

    /// One worker exercises every cadence property in sequence.
    ///
    /// Deliberately a single test rather than several: each spawned worker
    /// builds its own `macmon::Sampler`, and two of them sampling IOReport
    /// concurrently under a loaded test run is flaky. One worker, staged
    /// assertions.
    #[test]
    fn the_worker_delivers_samples_responds_to_cadence_and_parks() {
        use crate::cadence::TrayState;
        use std::time::Duration;

        // A slow cadence: one short measurement window, then a ~9s gap.
        let cadence = Arc::new(Cadence::new(10_000));
        let handle = super::spawn_sampler(Arc::clone(&cadence));
        assert!(
            wait_for_sample(&handle, Duration::from_secs(6)).is_some(),
            "no sample ever crossed the channel"
        );

        // 1. Speeding up the cadence must not wait out the old interval.
        cadence.set_state(TrayState {
            menu_open: true,
            on_battery: true,
            low_power: true,
            display_asleep: false,
        });
        let took = wait_for_sample(&handle, Duration::from_secs(8))
            .expect("no sample after the cadence was sped up");
        assert!(
            took < Duration::from_secs(4),
            "cadence change took {took:?}; the worker waited out the old interval"
        );

        // 2. Parking must actually stop the worker, not merely flip a flag.
        // The flag round-trip alone is covered by cadence's own tests;
        // asserting only that here would leave the suite with no coverage of
        // the behaviour that matters.
        // park() cannot abort a measurement already inside its window — it
        // only stops the next one — so wait out a full window before
        // draining, or the in-flight sample lands after the drain and looks
        // like the worker ignored the park.
        cadence.park();
        std::thread::sleep(Duration::from_millis(
            (crate::cadence::SAMPLE_WINDOW_MS + 800) as u64,
        ));
        while handle.try_recv().is_some() {} // drain whatever was in flight
        std::thread::sleep(Duration::from_millis(
            (crate::cadence::SAMPLE_WINDOW_MS + 800) as u64,
        ));
        assert!(
            handle.try_recv().is_none(),
            "worker kept sampling after park"
        );

        // 3. ...and unparking must wake it again. This is the only coverage
        // in the suite of the notify path where a lost wakeup would live.
        cadence.unpark();
        assert!(
            wait_for_sample(&handle, Duration::from_secs(6)).is_some(),
            "worker never resumed after unpark — lost wakeup"
        );
        cadence.park();
    }
}
