use std::process::Command;

#[test]
fn watch_emits_one_json_object_per_line_and_stops_at_count() {
    let out = Command::new(env!("CARGO_BIN_EXE_vitals"))
        .args(["watch", "-i", "1", "-n", "3"])
        .output()
        .expect("failed to run vitals watch");
    assert!(out.status.success());

    let text = String::from_utf8(out.stdout).unwrap();
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    assert_eq!(lines.len(), 3, "expected 3 NDJSON lines, got {}", lines.len());

    for line in lines {
        let v: serde_json::Value = serde_json::from_str(line)
            .unwrap_or_else(|e| panic!("line was not JSON ({e}): {line}"));
        assert_eq!(v["schema_version"], 1);
        assert!(v["cpu_pct"].is_number());
        assert!(!line.contains('\n'), "NDJSON lines must not be pretty-printed");
    }
}
