//! The Objective-C object that owns the status item, the menu and the timer.
//!
//! Everything here runs on the AppKit main thread. The only work it does per
//! tick is a non-blocking `try_recv` and, when a string actually changed,
//! one `setTitle:`.
//!
//! Three properties of `vitals-core`'s sampler shape this file:
//!
//! 1. `SamplerHandle` is `Send` but **not** `Sync` — it owns an
//!    `mpsc::Receiver`. It therefore lives in an ivar of a `MainThreadOnly`
//!    class, which is neither `Send` nor `Sync`, rather than in a `static`.
//! 2. The worker has no way to push into the run loop, so the handle is
//!    polled from an `NSTimer`. That timer is driven by `Cadence`: it is
//!    invalidated outright while the worker is parked, and otherwise ticks
//!    no faster than the worker samples. A fixed 1 Hz timer would spend back
//!    exactly the idle budget parking exists to protect.
//! 3. Once the worker dies there is no restart path and every later
//!    `try_recv` returns the same error. It is latched: AppKit is touched
//!    once, and the poll timer is torn down for good.

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{define_class, msg_send, sel, DefinedClass, MainThreadOnly};
use objc2_app_kit::{
    NSApplication, NSFont, NSMenu, NSMenuDelegate, NSMenuItem, NSStatusBar, NSStatusItem,
    NSWorkspace, NSWorkspaceScreensDidSleepNotification, NSWorkspaceScreensDidWakeNotification,
};
use objc2_foundation::{
    MainThreadMarker, NSNotification, NSNotificationCenter,
    NSProcessInfoPowerStateDidChangeNotification, NSString, NSTimer,
};
use std::cell::{Cell, RefCell};
use std::sync::Arc;
use vitals_core::cadence::{interval_for, Cadence, TrayState};
use vitals_core::host::{load_avg, uptime_s};
use vitals_core::sample::{spawn_sampler, Sample, SamplerHandle};
use vitals_core::schema::{build_snapshot, now_rfc3339, Snapshot, SnapshotInputs};
use vitals_core::sysctl::mem_pressure_level;
use vitals_core::thermal::{low_power_mode, on_battery, thermal_state};

use super::status_item::{format_title, menu_lines};

/// `NSVariableStatusItemLength`. objc2-app-kit 0.3 does not re-export the
/// AppKit constant, so it is spelled out here.
const NS_VARIABLE_STATUS_ITEM_LENGTH: f64 = -1.0;

/// Number of informational rows above the separator.
const ROWS: usize = 4;

/// Floor on the poll period. The worker never produces samples faster than
/// once a second (`cadence::SAMPLE_WINDOW_MS`), so polling faster only burns
/// wakeups.
const MIN_POLL_MS: u32 = 1_000;

/// The exact string `vitals_core::sample::SamplerHandle::try_recv` returns
/// once the worker thread is gone. It is the only error that never clears,
/// so it — and only it — retires the poll timer. Any other error is
/// transient: the worker keeps looping and the next sample will clear it.
const SAMPLER_STOPPED: &str = "sampler thread stopped";

/// Shown in the menu bar before the first sample lands.
const PLACEHOLDER_TITLE: &str = "…";

/// Shown in the menu bar once the worker is gone for good. Deliberately not
/// a number: stale numbers that never move again are worse than no numbers.
const FAILED_TITLE: &str = "⚠";

pub struct Ivars {
    status_item: Retained<NSStatusItem>,
    menu: Retained<NSMenu>,
    handle: SamplerHandle,
    cadence: Arc<Cadence>,
    state: Cell<TrayState>,
    /// Last string handed to AppKit, so an unchanged tick costs nothing.
    last_title: RefCell<String>,
    last_rows: RefCell<[String; ROWS]>,
    last_snapshot: RefCell<Option<Snapshot>>,
    last_error: RefCell<Option<String>>,
    timer: RefCell<Option<Retained<NSTimer>>>,
    /// Period the live timer was created with; `None` means no timer.
    timer_period_ms: Cell<Option<u32>>,
    /// Latch for a worker that will never produce another sample.
    sampler_dead: Cell<bool>,
}

define_class!(
    // SAFETY:
    // - NSObject has no subclassing requirements.
    // - `Controller` does not implement `Drop`.
    #[unsafe(super(NSObject))]
    // Every ivar reaches AppKit, and `SamplerHandle` is not `Sync`; this
    // object must never leave the main thread.
    #[thread_kind = MainThreadOnly]
    #[name = "VitalsController"]
    #[ivars = Ivars]
    pub struct Controller;

    impl Controller {
        #[unsafe(method(tick:))]
        fn tick(&self, _timer: *mut NSTimer) {
            if self.ivars().sampler_dead.get() {
                return; // the timer is already gone; belt and braces
            }
            // `try_recv` already returns the newest sample and drops any
            // backlog, so there is deliberately no draining loop here.
            let Some(result) = self.ivars().handle.try_recv() else {
                return;
            };
            match result {
                Ok(sample) => self.apply_sample(sample),
                Err(e) => self.apply_error(e),
            }
        }

        #[unsafe(method(quit:))]
        fn quit(&self, _sender: *mut AnyObject) {
            NSApplication::sharedApplication(MainThreadMarker::from(self)).terminate(None);
        }

        #[unsafe(method(displaySlept:))]
        fn display_slept(&self, _n: *mut NSNotification) {
            self.mutate_state(|s| s.display_asleep = true);
        }

        #[unsafe(method(displayWoke:))]
        fn display_woke(&self, _n: *mut NSNotification) {
            self.mutate_state(|s| s.display_asleep = false);
        }

        #[unsafe(method(powerChanged:))]
        fn power_changed(&self, _n: *mut NSNotification) {
            // Read both: unplugging fires this notification, and the battery
            // tier is what the cadence table is mostly there for.
            let low = low_power_mode();
            let batt = on_battery();
            self.mutate_state(|s| {
                s.low_power = low;
                s.on_battery = batt;
            });
        }
    }

    unsafe impl NSObjectProtocol for Controller {}

    unsafe impl NSMenuDelegate for Controller {
        #[unsafe(method(menuWillOpen:))]
        fn menu_will_open(&self, _menu: &NSMenu) {
            self.mutate_state(|s| s.menu_open = true);
            // Paint what we already have rather than waiting a tick.
            self.write_rows();
        }

        #[unsafe(method(menuDidClose:))]
        fn menu_did_close(&self, _menu: &NSMenu) {
            self.mutate_state(|s| s.menu_open = false);
        }
    }
);

impl Controller {
    pub fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let state = TrayState {
            menu_open: false,
            on_battery: on_battery(),
            low_power: low_power_mode(),
            display_asleep: false,
        };
        let cadence = Arc::new(Cadence::new(interval_for(state)));
        let handle = spawn_sampler(Arc::clone(&cadence));

        let status_item =
            NSStatusBar::systemStatusBar().statusItemWithLength(NS_VARIABLE_STATUS_ITEM_LENGTH);
        if let Some(button) = status_item.button(mtm) {
            // Monospaced digits: proportional digits make the item jitter as
            // numbers change, which reads as jank.
            let size = NSFont::systemFontSize();
            button.setFont(Some(&NSFont::monospacedDigitSystemFontOfSize_weight(
                size, 0.0,
            )));
            button.setTitle(&NSString::from_str(PLACEHOLDER_TITLE));
        }

        let menu = NSMenu::new(mtm);
        // The informational rows carry no action, and AppKit's automatic
        // enabling would grey them out anyway; make it explicit so Quit's
        // state does not depend on responder-chain lookup either.
        menu.setAutoenablesItems(false);
        for _ in 0..ROWS {
            let item = NSMenuItem::new(mtm);
            item.setEnabled(false);
            menu.addItem(&item);
        }
        menu.addItem(&NSMenuItem::separatorItem(mtm));

        let ivars = Ivars {
            status_item: status_item.clone(),
            menu: menu.clone(),
            handle,
            cadence,
            state: Cell::new(state),
            last_title: RefCell::new(PLACEHOLDER_TITLE.to_string()),
            last_rows: RefCell::new(std::array::from_fn(|_| String::new())),
            last_snapshot: RefCell::new(None),
            last_error: RefCell::new(None),
            timer: RefCell::new(None),
            timer_period_ms: Cell::new(None),
            sampler_dead: Cell::new(false),
        };

        let this = Self::alloc(mtm).set_ivars(ivars);
        let this: Retained<Self> = unsafe { msg_send![super(this), init] };

        let quit = NSMenuItem::new(mtm);
        quit.setTitle(&NSString::from_str("Quit vitals"));
        quit.setKeyEquivalent(&NSString::from_str("q"));
        quit.setEnabled(true);
        // SAFETY: `this` responds to `quit:`, defined above.
        unsafe {
            quit.setTarget(Some(&this));
            quit.setAction(Some(sel!(quit:)));
        }
        menu.addItem(&quit);

        menu.setDelegate(Some(ProtocolObject::from_ref(&*this)));
        status_item.setMenu(Some(&menu));

        this.observe_notifications();
        this.sync_timer();
        this
    }

    /// Register for the two events that change the sampling cadence.
    ///
    /// Screen sleep/wake are posted on `NSWorkspace`'s own centre; the
    /// low-power-mode notification is posted on the default centre, so it
    /// must not be registered on the workspace centre or it will never fire.
    fn observe_notifications(&self) {
        let workspace = NSWorkspace::sharedWorkspace().notificationCenter();
        let default = NSNotificationCenter::defaultCenter();
        // SAFETY: `self` responds to each selector, and every name is the
        // framework's own constant.
        unsafe {
            workspace.addObserver_selector_name_object(
                self,
                sel!(displaySlept:),
                Some(NSWorkspaceScreensDidSleepNotification),
                None,
            );
            workspace.addObserver_selector_name_object(
                self,
                sel!(displayWoke:),
                Some(NSWorkspaceScreensDidWakeNotification),
                None,
            );
            default.addObserver_selector_name_object(
                self,
                sel!(powerChanged:),
                Some(NSProcessInfoPowerStateDidChangeNotification),
                None,
            );
        }
    }

    fn mutate_state(&self, f: impl FnOnce(&mut TrayState)) {
        let mut state = self.ivars().state.get();
        f(&mut state);
        self.ivars().state.set(state);
        self.ivars().cadence.set_state(state);
        // `set_state` may have parked or unparked the worker, and will have
        // changed the interval; the poll timer follows it.
        self.sync_timer();
    }

    /// How often the run loop should poll the channel, or `None` for "not at
    /// all". This is the whole of integration fact 2: a parked worker gets no
    /// timer, and an unparked one gets a timer no faster than its own cadence.
    fn desired_poll_ms(&self) -> Option<u32> {
        if self.ivars().sampler_dead.get() || self.ivars().cadence.is_parked() {
            return None;
        }
        if self.ivars().last_snapshot.borrow().is_none() {
            // Nothing on screen yet — poll fast enough to paint the first
            // sample promptly, then settle onto the cadence below.
            return Some(MIN_POLL_MS);
        }
        Some(self.ivars().cadence.interval_ms().max(MIN_POLL_MS))
    }

    fn sync_timer(&self) {
        let desired = self.desired_poll_ms();
        if desired == self.ivars().timer_period_ms.get() {
            return;
        }
        if let Some(timer) = self.ivars().timer.borrow_mut().take() {
            timer.invalidate();
        }
        if let Some(ms) = desired {
            let seconds = f64::from(ms) / 1000.0;
            // SAFETY: `self` responds to `tick:`; there is no user info.
            let timer = unsafe {
                NSTimer::scheduledTimerWithTimeInterval_target_selector_userInfo_repeats(
                    seconds,
                    self,
                    sel!(tick:),
                    None,
                    true,
                )
            };
            // Let the kernel coalesce this wakeup with others already due.
            timer.setTolerance(seconds * 0.25);
            *self.ivars().timer.borrow_mut() = Some(timer);
        }
        self.ivars().timer_period_ms.set(desired);
    }

    fn apply_sample(&self, sample: Sample) {
        let snapshot = build_snapshot(SnapshotInputs {
            metrics: &sample.metrics,
            host: sample.host.clone(),
            sample_ms: sample.sample_ms,
            sampled_at: now_rfc3339(),
            mem_pressure: mem_pressure_level(),
            thermal_state: thermal_state(),
            load_avg: load_avg(),
            uptime_s: uptime_s(),
        });

        *self.ivars().last_error.borrow_mut() = None;
        self.set_title(&format_title(&snapshot));
        *self.ivars().last_snapshot.borrow_mut() = Some(snapshot);
        if self.ivars().state.get().menu_open {
            self.write_rows();
        }
        // The first sample retires the fast start-up poll.
        self.sync_timer();
    }

    /// Handle an error from the worker exactly once.
    ///
    /// A dead worker repeats the same error on every poll forever, so both
    /// paths are latched: AppKit is touched only when the message actually
    /// changes, and `SAMPLER_STOPPED` additionally retires the timer,
    /// because nothing will ever arrive again.
    fn apply_error(&self, message: String) {
        if message == SAMPLER_STOPPED {
            if self.ivars().sampler_dead.get() {
                return; // already latched; say nothing further, ever
            }
            self.ivars().sampler_dead.set(true);
            // The worker sends its real failure just before it exits, so a
            // message already latched is the *cause* and this one is only
            // the consequence. Keep the cause.
            let unexplained = self.ivars().last_error.borrow().is_none();
            if unexplained {
                *self.ivars().last_error.borrow_mut() = Some(message);
            }
            self.set_title(FAILED_TITLE);
            self.sync_timer(); // tears the poll timer down for good
        } else {
            let unchanged = self.ivars().last_error.borrow().as_deref() == Some(message.as_str());
            if unchanged {
                return;
            }
            // A transient error keeps the last good numbers in the menu bar —
            // they are seconds old, not wrong — and explains itself in the
            // menu, where the next successful sample will clear it.
            *self.ivars().last_error.borrow_mut() = Some(message);
        }
        if self.ivars().state.get().menu_open {
            self.write_rows();
        }
    }

    fn set_title(&self, title: &str) {
        if *self.ivars().last_title.borrow() == title {
            return; // budget rule: only touch AppKit when the string changed
        }
        if let Some(button) = self.ivars().status_item.button(MainThreadMarker::from(self)) {
            button.setTitle(&NSString::from_str(title));
        }
        *self.ivars().last_title.borrow_mut() = title.to_string();
    }

    /// The rows the dropdown should currently show.
    fn rows(&self) -> [String; ROWS] {
        if let Some(error) = self.ivars().last_error.borrow().clone() {
            let mut rows: [String; ROWS] = std::array::from_fn(|_| String::new());
            rows[0] = format!("⚠  {error}");
            return rows;
        }
        match self.ivars().last_snapshot.borrow().as_ref() {
            Some(snapshot) => menu_lines(snapshot),
            None => {
                let mut rows: [String; ROWS] = std::array::from_fn(|_| String::new());
                rows[0] = "sampling…".to_string();
                rows
            }
        }
    }

    /// Push the rows into the menu, skipping the work when nothing changed.
    /// Only ever called with the menu open — budget rule 3.
    fn write_rows(&self) {
        let rows = self.rows();
        if *self.ivars().last_rows.borrow() == rows {
            return;
        }
        for (index, line) in rows.iter().enumerate() {
            if let Some(item) = self.ivars().menu.itemAtIndex(index as isize) {
                item.setTitle(&NSString::from_str(line));
                item.setHidden(line.is_empty());
            }
        }
        *self.ivars().last_rows.borrow_mut() = rows;
    }
}
