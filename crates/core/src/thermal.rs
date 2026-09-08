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
