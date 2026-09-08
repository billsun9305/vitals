use std::process::Command;

#[test]
fn pressure_emits_a_verdict_with_a_summary_sentence() {
    let out = Command::new(env!("CARGO_BIN_EXE_vitals"))
        .arg("pressure")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();

    assert_eq!(v["schema_version"], 1);
    assert!(["nominal", "warning", "critical"].contains(&v["state"].as_str().unwrap()));
    assert!(v["history"].is_boolean());
    assert!(v["reasons"].is_array());
    assert!(v["suspects"].is_array());
    let summary = v["summary"].as_str().unwrap();
    assert!(
        summary.ends_with('.'),
        "summary should be a sentence: {summary}"
    );
    assert!(!summary.is_empty());
}

#[test]
fn a_second_run_has_history_because_the_first_stored_a_baseline() {
    let bin = env!("CARGO_BIN_EXE_vitals");
    Command::new(bin).arg("pressure").output().unwrap();
    let out = Command::new(bin).arg("pressure").output().unwrap();
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["history"], true);
}

#[test]
fn exit_code_flag_reports_the_state() {
    let out = Command::new(env!("CARGO_BIN_EXE_vitals"))
        .args(["pressure", "--exit-code"])
        .output()
        .unwrap();
    // 0 nominal, 3 warning, 4 critical — never 1, which means a real error.
    let code = out.status.code().unwrap();
    assert!([0, 3, 4].contains(&code), "unexpected exit code {code}");

    // Membership alone would accept a swapped mapping, since 3 and 4 are
    // both in the set either way. Cross-check the code against the state
    // this same run reported.
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let expected = match v["state"].as_str().unwrap() {
        "nominal" => 0,
        "warning" => 3,
        "critical" => 4,
        other => panic!("unknown state {other:?}"),
    };
    assert_eq!(
        code, expected,
        "state {:?} should exit {expected}, exited {code}",
        v["state"]
    );
}
