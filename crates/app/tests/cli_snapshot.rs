use std::process::Command;

fn run(args: &[&str]) -> serde_json::Value {
    let out = Command::new(env!("CARGO_BIN_EXE_vitals"))
        .args(args)
        .output()
        .expect("failed to run vitals");
    assert!(out.status.success(), "vitals {args:?} exited {:?}\nstderr: {}",
            out.status.code(), String::from_utf8_lossy(&out.stderr));
    serde_json::from_slice(&out.stdout).expect("stdout was not valid JSON")
}

#[test]
fn snapshot_emits_the_documented_contract() {
    let v = run(&["snapshot", "--interval", "150"]);
    assert_eq!(v["schema_version"], 1);
    assert_eq!(v["sample_ms"], 150);
    assert!(v["sampled_at"].as_str().unwrap().ends_with('Z'));

    for key in ["cpu_pct", "cpu_scaled_pct", "ecpu_pct", "pcpu_pct", "gpu_pct",
                "power_total_w", "temp_cpu_c"] {
        assert!(v[key].is_number(), "{key} missing or not a number: {v}");
    }
    for key in ["mem_total_mb", "mem_used_mb", "swap_total_mb", "swap_used_mb", "uptime_s"] {
        assert!(v[key].is_u64(), "{key} missing or not an integer: {v}");
    }
    assert!(v["mem_total_mb"].as_u64().unwrap() >= 8192, "at least 8 GB expected");
    assert!(v["host"]["chip"].as_str().unwrap().contains("Apple"));
    assert_eq!(v["load_avg"].as_array().unwrap().len(), 3);
    assert!(v["cores"].as_array().unwrap().len() >= 2);
    assert!(["normal", "warning", "critical", "unknown"]
        .contains(&v["mem_pressure"].as_str().unwrap()));
}

#[test]
fn json_is_the_default_and_json_flag_is_a_no_op() {
    let a = run(&["snapshot", "--interval", "120"]);
    let b = run(&["snapshot", "--interval", "120", "--json"]);
    assert_eq!(a["schema_version"], b["schema_version"]);
}

#[test]
fn no_null_ever_appears_in_the_output() {
    let out = Command::new(env!("CARGO_BIN_EXE_vitals"))
        .args(["snapshot", "--interval", "120"])
        .output()
        .unwrap();
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(!text.contains("null"), "schema rule violated: {text}");
}
