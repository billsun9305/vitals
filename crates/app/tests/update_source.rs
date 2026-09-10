//! `vitals --update-source <bad>` must fail before the tray starts.

use std::process::Command;

#[test]
fn a_non_loopback_update_source_is_refused_before_the_tray_starts() {
    let out = Command::new(env!("CARGO_BIN_EXE_vitals"))
        .args(["--update-source", "https://example.com/"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("update source must be"), "{stderr}");
}
