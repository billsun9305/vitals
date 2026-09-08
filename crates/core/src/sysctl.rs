//! The kernel's own memory-pressure verdict.
//!
//! `kern.memorystatus_vm_pressure_level` returns the
//! `dispatch_source_memorypressure_flags_t` values: 1 normal, 2 warning,
//! 4 critical. It is unreadable inside an App Sandbox, which is one more
//! reason `vitals` is not a sandboxed app.

use crate::schema::PressureLevel;

fn level_from_raw(v: libc::c_int) -> PressureLevel {
    match v {
        1 => PressureLevel::Normal,
        2 => PressureLevel::Warning,
        4 => PressureLevel::Critical,
        _ => PressureLevel::Unknown,
    }
}

pub fn mem_pressure_level() -> PressureLevel {
    let name = c"kern.memorystatus_vm_pressure_level";
    let mut value: libc::c_int = 0;
    let mut size = std::mem::size_of::<libc::c_int>();
    // SAFETY: `name` is a NUL-terminated C string, `value` is a live
    // c_int, and `size` correctly describes it. sysctlbyname writes at
    // most `size` bytes and updates `size` with what it wrote.
    let rc = unsafe {
        libc::sysctlbyname(
            name.as_ptr(),
            &mut value as *mut libc::c_int as *mut libc::c_void,
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    if rc != 0 {
        return PressureLevel::Unknown;
    }
    level_from_raw(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::PressureLevel;

    #[test]
    fn kernel_reports_a_known_pressure_level() {
        let level = mem_pressure_level();
        assert_ne!(
            level,
            PressureLevel::Unknown,
            "kern.memorystatus_vm_pressure_level should be readable outside a sandbox"
        );
    }

    #[test]
    fn raw_values_map_to_the_documented_levels() {
        assert_eq!(level_from_raw(1), PressureLevel::Normal);
        assert_eq!(level_from_raw(2), PressureLevel::Warning);
        assert_eq!(level_from_raw(4), PressureLevel::Critical);
        assert_eq!(level_from_raw(3), PressureLevel::Unknown);
        assert_eq!(level_from_raw(0), PressureLevel::Unknown);
    }
}
