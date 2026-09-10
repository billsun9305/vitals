//! Two questions for the Security framework: which Team ID signed the
//! running bundle, and does a staged bundle carry a valid Developer ID
//! signature from that same team.
//!
//! This is what makes an update authentic rather than merely intact. The
//! SHA-256 in `SHA256SUMS` proves the tarball is the one the release
//! published; only the signature proves the release came from the person
//! holding the certificate, which a compromised GitHub account does not.
//! An ad-hoc copy (a local build, or a release built without the secrets)
//! has no Team ID, and the installer skips the check for it — `SECURITY.md`
//! says so.

use std::ffi::c_void;
use std::path::Path;
use std::ptr::NonNull;

use objc2_core_foundation::{CFDictionary, CFRetained, CFString, CFType, CFURL};
use objc2_security::{
    kSecCSSigningInformation, kSecCodeInfoTeamIdentifier, SecCSFlags, SecCode, SecRequirement,
    SecStaticCode,
};

/// The designated requirement an update must satisfy: signed through
/// Apple's Developer ID chain (`anchor apple generic`) by a leaf
/// certificate whose organisational unit is this team.
pub fn requirement_for(team_id: &str) -> String {
    format!("anchor apple generic and certificate leaf[subject.OU] = \"{team_id}\"")
}

/// The Team ID in the running process's own code signature, or `None`
/// when there is none — an ad-hoc signature, or no signature at all.
pub fn team_id_of_self() -> Option<String> {
    let mut code: *mut SecCode = std::ptr::null_mut();
    // SAFETY: `code` is a valid out-slot for one pointer. The framework
    // writes it only on success, which is checked before it is read.
    let status = unsafe { SecCode::copy_self(SecCSFlags::DefaultFlags, NonNull::from(&mut code)) };
    if status != 0 {
        return None;
    }
    // SAFETY: on success the slot holds a +1 reference this scope now owns.
    let code = unsafe { CFRetained::from_raw(NonNull::new(code)?) };

    let mut static_code: *const SecStaticCode = std::ptr::null();
    // SAFETY: as above.
    let status =
        unsafe { code.copy_static_code(SecCSFlags::DefaultFlags, NonNull::from(&mut static_code)) };
    if status != 0 {
        return None;
    }
    // SAFETY: as above; `cast_mut` changes only the pointer's type.
    let static_code = unsafe { CFRetained::from_raw(NonNull::new(static_code.cast_mut())?) };
    team_id_of(&static_code)
}

/// The `teamid` entry of a code object's signing information.
fn team_id_of(code: &SecStaticCode) -> Option<String> {
    let mut info: *const CFDictionary = std::ptr::null();
    // SAFETY: `info` is a valid out-slot; the signing-information flag asks
    // for the certificate-derived entries, which is where the Team ID is.
    let status = unsafe {
        SecCode::copy_signing_information(
            code,
            SecCSFlags(kSecCSSigningInformation),
            NonNull::from(&mut info),
        )
    };
    if status != 0 {
        return None;
    }
    // SAFETY: on success the slot holds a +1 reference this scope now owns.
    let info = unsafe { CFRetained::from_raw(NonNull::new(info.cast_mut())?) };
    // SAFETY: the key is the framework's own constant, and the dictionary's
    // keys are CFStrings, so `CFDictionaryGetValue` with it is the
    // documented lookup. The value is borrowed from `info`, alive until
    // this function returns.
    let value = unsafe {
        let key: &CFString = kSecCodeInfoTeamIdentifier;
        info.value((key as *const CFString).cast::<c_void>())
    };
    let value = NonNull::new(value.cast_mut())?;
    // SAFETY: a non-null value in this dictionary is a CF object, and
    // `downcast_ref` checks that it is a CFString rather than assuming.
    let team = unsafe { value.cast::<CFType>().as_ref() }.downcast_ref::<CFString>()?;
    Some(team.to_string())
}

/// Does the bundle at `bundle` carry a valid signature satisfying
/// `requirement_for(team_id)`? `Err` carries the reason, `OSStatus`
/// included, for the failure alert.
pub fn verify_team(bundle: &Path, team_id: &str) -> Result<(), String> {
    let url = CFURL::from_file_path(bundle)
        .ok_or_else(|| format!("not a file path: {}", bundle.display()))?;

    let mut code: *const SecStaticCode = std::ptr::null();
    // SAFETY: `url` is a live CFURL and `code` a valid out-slot; checked
    // before read.
    let status = unsafe {
        SecStaticCode::create_with_path(&url, SecCSFlags::DefaultFlags, NonNull::from(&mut code))
    };
    if status != 0 {
        return Err(format!(
            "could not read the code signature of {}: OSStatus {status}",
            bundle.display()
        ));
    }
    // SAFETY: on success the slot holds a +1 reference this scope now owns.
    let code =
        unsafe { CFRetained::from_raw(NonNull::new(code.cast_mut()).ok_or("no code object")?) };

    let text = CFString::from_str(&requirement_for(team_id));
    let mut requirement: *mut SecRequirement = std::ptr::null_mut();
    // SAFETY: `text` is a live CFString and `requirement` a valid out-slot;
    // checked before read.
    let status = unsafe {
        SecRequirement::create_with_string(
            &text,
            SecCSFlags::DefaultFlags,
            NonNull::from(&mut requirement),
        )
    };
    if status != 0 {
        return Err(format!(
            "could not compile the code requirement: OSStatus {status}"
        ));
    }
    // SAFETY: as above.
    let requirement =
        unsafe { CFRetained::from_raw(NonNull::new(requirement).ok_or("no requirement object")?) };

    // SAFETY: both objects are alive for the call.
    let status = unsafe { code.check_validity(SecCSFlags::DefaultFlags, Some(&requirement)) };
    if status == 0 {
        Ok(())
    } else {
        Err(format!(
            "{} does not satisfy `{}`: OSStatus {status}",
            bundle.display(),
            requirement_for(team_id)
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_requirement_is_the_developer_id_designated_form() {
        assert_eq!(
            requirement_for("4WD8D5Y6NF"),
            "anchor apple generic and certificate leaf[subject.OU] = \"4WD8D5Y6NF\""
        );
    }

    #[test]
    fn a_test_binary_has_no_team_id() {
        // cargo's test binaries are ad-hoc signed by the linker: a valid
        // signature with no certificate chain and therefore no Team ID.
        // This is also the case the installer treats as "skip the check".
        assert_eq!(team_id_of_self(), None);
    }

    #[test]
    fn a_missing_bundle_fails_to_verify() {
        let err = verify_team(
            std::path::Path::new("/nonexistent/Vitals.app"),
            "4WD8D5Y6NF",
        )
        .unwrap_err();
        assert!(!err.is_empty());
    }

    #[test]
    fn an_ad_hoc_binary_does_not_satisfy_a_team_requirement() {
        let me = std::env::current_exe().unwrap();
        let err = verify_team(&me, "4WD8D5Y6NF").unwrap_err();
        assert!(err.contains("OSStatus"), "{err}");
    }
}
