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
        Ok(Sample { metrics, host: self.host.clone(), sample_ms: interval_ms })
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
    /// Non-blocking. Returns `None` when no new sample has arrived; the
    /// AppKit main thread must never block on this.
    pub fn try_recv(&self) -> Option<Result<Sample, String>> {
        match self.rx.try_recv() {
            Ok(v) => Some(v),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => Some(Err("sampler thread stopped".to_string())),
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
                match session.next(interval) {
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
        assert!(s.metrics.memory.ram_total > 0, "ram_total should be non-zero");
        assert!(!s.host.chip.is_empty());
        assert!((0.0..=1.01).contains(&s.metrics.cpu_active_ratio),
                "active ratio out of range: {}", s.metrics.cpu_active_ratio);
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
    fn the_worker_delivers_samples_and_parks_on_demand() {
        use crate::cadence::Cadence;
        use std::sync::Arc;

        let cadence = Arc::new(Cadence::new(100));
        let handle = super::spawn_sampler(Arc::clone(&cadence));

        let mut got = None;
        for _ in 0..50 {
            if let Some(v) = handle.try_recv() {
                got = Some(v.expect("worker reported an error"));
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        assert!(got.is_some(), "no sample arrived within 2.5s");

        cadence.park();
        assert!(cadence.is_parked());
        cadence.unpark();
    }
}
