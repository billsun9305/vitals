//! Start at Login through `SMAppService`, the API a bundled app uses to
//! register itself as a login item (macOS 13+; our floor is 14). No
//! LaunchAgent plist, no helper: the registration names the bundle, so it
//! survives the updater swapping the bundle and goes away with the app.
//!
//! `menu_state` is the pure part and is unit-tested. The calls into
//! ServiceManagement are verified by running them — the manual test in
//! CONTRIBUTING.md — because there is nothing to assert on outside a
//! bundle.

use objc2_foundation::{NSString, NSUserDefaults};
use objc2_service_management::{SMAppService, SMAppServiceStatus};

use crate::cli::LoginItemAction;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoginStatus {
    Enabled,
    NotRegistered,
    /// Registered, but macOS wants the user to approve it in System
    /// Settings before it takes effect.
    RequiresApproval,
    /// Not a bundle, or the bundle cannot be a login item.
    NotFound,
}

/// The `NSUserDefaults` key that says `register_once` has run.
const REGISTERED_KEY: &str = "registeredAtLogin";

pub fn status() -> LoginStatus {
    // SAFETY: `mainAppService` and `status` have no preconditions; outside
    // a bundle they report `NotFound`.
    let raw = unsafe { SMAppService::mainAppService().status() };
    if raw == SMAppServiceStatus::Enabled {
        LoginStatus::Enabled
    } else if raw == SMAppServiceStatus::RequiresApproval {
        LoginStatus::RequiresApproval
    } else if raw == SMAppServiceStatus::NotFound {
        LoginStatus::NotFound
    } else {
        LoginStatus::NotRegistered
    }
}

/// Register (`true`) or unregister (`false`) this bundle. Registering an
/// already-registered bundle succeeds.
pub fn set(on: bool) -> Result<(), String> {
    // SAFETY: no preconditions; a failure comes back as the error.
    let result = unsafe {
        let service = SMAppService::mainAppService();
        if on {
            service.registerAndReturnError()
        } else {
            service.unregisterAndReturnError()
        }
    };
    result.map_err(|e| e.localizedDescription().to_string())
}

/// Open System Settings at Login Items, for the `RequiresApproval` state.
pub fn open_settings() {
    // SAFETY: no preconditions.
    unsafe { SMAppService::openSystemSettingsLoginItems() }
}

/// First bundled launch only: register, and remember having done so, so
/// a user who later turns it off is not enrolled again next launch.
pub fn register_once() {
    let defaults = NSUserDefaults::standardUserDefaults();
    let key = NSString::from_str(REGISTERED_KEY);
    if defaults.boolForKey(&key) {
        return;
    }
    match set(true) {
        Ok(()) => defaults.setBool_forKey(true, &key),
        Err(e) => eprintln!("vitals: could not register as a login item: {e}"),
    }
}

/// Pure: `(title, checked, enabled)` for the *Start at Login* menu item.
pub fn menu_state(status: LoginStatus) -> (&'static str, bool, bool) {
    match status {
        LoginStatus::Enabled => ("Start at Login", true, true),
        LoginStatus::NotRegistered => ("Start at Login", false, true),
        LoginStatus::RequiresApproval => {
            ("Start at Login — approve in System Settings…", false, true)
        }
        LoginStatus::NotFound => ("Start at Login", false, false),
    }
}

/// The hidden `vitals login-item` verb.
pub fn run(action: LoginItemAction) -> Result<(), String> {
    match action {
        LoginItemAction::On => set(true),
        LoginItemAction::Off => set(false),
        LoginItemAction::Status => {
            let word = match status() {
                LoginStatus::Enabled => "enabled",
                LoginStatus::NotRegistered => "not-registered",
                LoginStatus::RequiresApproval => "requires-approval",
                LoginStatus::NotFound => "not-found",
            };
            println!("{word}");
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menu_state_covers_every_status() {
        assert_eq!(
            menu_state(LoginStatus::Enabled),
            ("Start at Login", true, true)
        );
        assert_eq!(
            menu_state(LoginStatus::NotRegistered),
            ("Start at Login", false, true)
        );
        assert_eq!(
            menu_state(LoginStatus::RequiresApproval),
            ("Start at Login — approve in System Settings…", false, true)
        );
        assert_eq!(
            menu_state(LoginStatus::NotFound),
            ("Start at Login", false, false)
        );
    }
}
