//! Adaptive sampling cadence — the single biggest lever on idle cost.
//!
//! Shared between the AppKit main thread (which writes state on menu and
//! power notifications) and the sampler worker (which reads the interval and
//! blocks on the condvar while parked).

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

/// How long a single measurement actually takes.
///
/// The sampling window and the gap between samples are deliberately
/// separate. `get_metrics` blocks for exactly the window it is given, so
/// sampling for the full interval would mean a 30s call on low power — and
/// an open menu could not take effect until it returned, defeating the rule
/// that an open menu wins over every power-saving state. Measure over a
/// short fixed window, then spend the remainder in an interruptible wait.
/// A second of IOReport deltas is as stable as thirty.
pub const SAMPLE_WINDOW_MS: u32 = 1_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrayState {
    pub menu_open: bool,
    pub on_battery: bool,
    pub low_power: bool,
    pub display_asleep: bool,
}

/// Milliseconds between samples for a given state. A sleeping display is not
/// represented here — it parks the worker instead (see `Cadence::set_state`).
pub fn interval_for(s: TrayState) -> u32 {
    if s.menu_open {
        1_000
    } else if s.low_power {
        30_000
    } else if s.on_battery {
        15_000
    } else {
        5_000
    }
}

pub struct Cadence {
    interval_ms: AtomicU32,
    parked: AtomicBool,
    lock: Mutex<()>,
    cv: Condvar,
}

impl Cadence {
    pub fn new(initial_ms: u32) -> Self {
        Self {
            interval_ms: AtomicU32::new(initial_ms),
            parked: AtomicBool::new(false),
            lock: Mutex::new(()),
            cv: Condvar::new(),
        }
    }

    pub fn interval_ms(&self) -> u32 {
        self.interval_ms.load(Ordering::Relaxed)
    }

    pub fn is_parked(&self) -> bool {
        self.parked.load(Ordering::Relaxed)
    }

    pub fn park(&self) {
        self.parked.store(true, Ordering::Relaxed);
        // Notify under the lock so a worker idling between samples wakes and
        // parks now, instead of finishing a wait of up to 30 seconds first.
        let _guard = self.lock.lock().unwrap();
        self.cv.notify_all();
    }

    pub fn unpark(&self) {
        self.parked.store(false, Ordering::Relaxed);
        let _guard = self.lock.lock().unwrap();
        self.cv.notify_all();
    }

    pub fn set_state(&self, s: TrayState) {
        self.interval_ms.store(interval_for(s), Ordering::Relaxed);
        if s.display_asleep {
            self.park();
        } else {
            self.unpark();
        }
    }

    /// Wait out the gap between samples, returning early if the worker is
    /// parked or the cadence changes. Called only by the worker.
    ///
    /// `scheduled_for` is the interval in force when this gap was chosen; if
    /// it changes underneath us the gap is stale, so return and let the
    /// worker re-read it. This is what makes an open menu take effect within
    /// one sample window rather than one full interval.
    pub fn wait_between_samples(&self, ms: u32, scheduled_for: u32) {
        if ms == 0 {
            return;
        }
        let deadline = Instant::now() + Duration::from_millis(ms as u64);
        let mut guard = self.lock.lock().unwrap();
        loop {
            if self.parked.load(Ordering::Relaxed)
                || self.interval_ms.load(Ordering::Relaxed) != scheduled_for
            {
                return;
            }
            let now = Instant::now();
            if now >= deadline {
                return;
            }
            let (g, _) = self.cv.wait_timeout(guard, deadline - now).unwrap();
            guard = g;
        }
    }

    /// Block until unparked. Called only by the worker.
    pub fn wait_while_parked(&self) {
        let mut guard = self.lock.lock().unwrap();
        while self.parked.load(Ordering::Relaxed) {
            guard = self.cv.wait(guard).unwrap();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interval_follows_the_documented_table() {
        assert_eq!(
            interval_for(TrayState {
                menu_open: true,
                on_battery: false,
                low_power: false,
                display_asleep: false
            }),
            1_000
        );
        assert_eq!(
            interval_for(TrayState {
                menu_open: false,
                on_battery: false,
                low_power: false,
                display_asleep: false
            }),
            5_000
        );
        assert_eq!(
            interval_for(TrayState {
                menu_open: false,
                on_battery: true,
                low_power: false,
                display_asleep: false
            }),
            15_000
        );
        assert_eq!(
            interval_for(TrayState {
                menu_open: false,
                on_battery: true,
                low_power: true,
                display_asleep: false
            }),
            30_000
        );
    }

    #[test]
    fn an_open_menu_wins_over_every_power_saving_state() {
        let s = TrayState {
            menu_open: true,
            on_battery: true,
            low_power: true,
            display_asleep: false,
        };
        assert_eq!(
            interval_for(s),
            1_000,
            "the user is looking at it; sample fast"
        );
    }

    #[test]
    fn parking_and_unparking_round_trips() {
        let c = Cadence::new(5_000);
        assert!(!c.is_parked());
        c.park();
        assert!(c.is_parked());
        c.unpark();
        assert!(!c.is_parked());
    }

    #[test]
    fn setting_the_state_updates_the_interval() {
        let c = Cadence::new(5_000);
        c.set_state(TrayState {
            menu_open: true,
            on_battery: false,
            low_power: false,
            display_asleep: false,
        });
        assert_eq!(c.interval_ms(), 1_000);
        c.set_state(TrayState {
            menu_open: false,
            on_battery: false,
            low_power: false,
            display_asleep: true,
        });
        assert!(
            c.is_parked(),
            "a sleeping display parks the worker instead of slowing it"
        );
    }
}
