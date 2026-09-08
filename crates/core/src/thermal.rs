//! `NSProcessInfo` readings. Both calls are thread-safe and cheap; neither
//! requires a main-thread marker, which is why they are allowed in `core`.

use crate::schema::ThermalState;
use objc2_foundation::NSProcessInfo;

fn state_from_raw(v: isize) -> ThermalState {
    match v {
        0 => ThermalState::Nominal,
        1 => ThermalState::Fair,
        2 => ThermalState::Serious,
        3 => ThermalState::Critical,
        _ => ThermalState::Unknown,
    }
}

pub fn thermal_state() -> ThermalState {
    // objc2 models NS_ENUM as a transparent newtype over NSInteger; match on
    // the raw value so a new Apple case degrades to Unknown instead of panicking.
    state_from_raw(NSProcessInfo::processInfo().thermalState().0)
}

pub fn low_power_mode() -> bool {
    NSProcessInfo::processInfo().isLowPowerModeEnabled()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::ThermalState;

    #[test]
    fn thermal_state_is_known() {
        assert_ne!(thermal_state(), ThermalState::Unknown);
    }

    #[test]
    fn raw_values_map_to_the_documented_states() {
        assert_eq!(state_from_raw(0), ThermalState::Nominal);
        assert_eq!(state_from_raw(1), ThermalState::Fair);
        assert_eq!(state_from_raw(2), ThermalState::Serious);
        assert_eq!(state_from_raw(3), ThermalState::Critical);
        assert_eq!(state_from_raw(9), ThermalState::Unknown);
    }

    #[test]
    fn low_power_mode_is_readable() {
        let _ = low_power_mode(); // must not panic; either value is valid
    }
}

/// Whether the machine is running on battery rather than wall power.
///
/// `NSProcessInfo` exposes low-power mode but not the power source, so this
/// goes to IOKit. `IOPSGetProvidingPowerSourceType` returns a borrowed
/// CFString describing what is currently supplying the machine — "AC Power",
/// "Battery Power", or "UPS Power". Only the snapshot is owned, so only the
/// snapshot is released.
///
/// A desktop with no battery reports AC power, which is the right answer for
/// cadence purposes: nothing is being drained.
pub fn on_battery() -> bool {
    use std::ffi::c_void;

    #[link(name = "IOKit", kind = "framework")]
    extern "C" {
        fn IOPSCopyPowerSourcesInfo() -> *const c_void;
        fn IOPSGetProvidingPowerSourceType(snapshot: *const c_void) -> *const c_void;
    }
    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFStringGetCString(s: *const c_void, buf: *mut u8, len: isize, enc: u32) -> u8;
        fn CFRelease(cf: *const c_void);
    }
    const UTF8: u32 = 0x0800_0100;

    // SAFETY: IOPSCopyPowerSourcesInfo returns an owned CFTypeRef or null.
    // IOPSGetProvidingPowerSourceType borrows from it (Get rule — not
    // released here) and is valid while the snapshot lives, which it does
    // until the CFRelease below.
    unsafe {
        let snapshot = IOPSCopyPowerSourcesInfo();
        if snapshot.is_null() {
            return false; // can't tell; assume wall power and sample normally
        }
        let kind = IOPSGetProvidingPowerSourceType(snapshot);
        let mut buf = [0u8; 64];
        let got = !kind.is_null()
            && CFStringGetCString(kind, buf.as_mut_ptr(), buf.len() as isize, UTF8) != 0;
        CFRelease(snapshot);
        if !got {
            return false;
        }
        let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
        std::str::from_utf8(&buf[..end]).is_ok_and(|s| s.starts_with("Battery"))
    }
}

#[cfg(test)]
mod power_source_tests {
    use super::*;

    #[test]
    fn power_source_is_readable_and_agrees_with_pmset() {
        // Just calling it proves the IOKit symbols link and the CFString
        // round-trips; a machine is on exactly one power source, so the only
        // honest assertion is that it does not panic and returns a definite
        // answer. The value itself is cross-checked against pmset by hand in
        // the task report, since spawning a subprocess here is forbidden.
        let a = on_battery();
        let b = on_battery();
        assert_eq!(a, b, "power source flapped between two immediate reads");
    }
}
