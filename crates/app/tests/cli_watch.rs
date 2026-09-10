use std::process::Command;

#[test]
fn watch_emits_one_json_object_per_line_and_stops_at_count() {
    vitals_core::skip_without_hardware!();
    let out = Command::new(env!("CARGO_BIN_EXE_vitals"))
        .args(["watch", "-i", "1", "-n", "3"])
        .output()
        .expect("failed to run vitals watch");
    assert!(out.status.success());

    let text = String::from_utf8(out.stdout).unwrap();
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    assert_eq!(
        lines.len(),
        3,
        "expected 3 NDJSON lines, got {}",
        lines.len()
    );

    for line in lines {
        let v: serde_json::Value = serde_json::from_str(line)
            .unwrap_or_else(|e| panic!("line was not JSON ({e}): {line}"));
        assert_eq!(v["schema_version"], 1);
        assert!(v["cpu_pct"].is_number());
        // `text.lines()` can never yield a string containing a newline, so
        // asserting that would prove nothing. What actually distinguishes
        // NDJSON from a pretty-printed record is that one whole object fits
        // on one line: a pretty record would arrive as `{` alone here.
        assert!(
            line.trim_start().starts_with('{') && line.trim_end().ends_with('}'),
            "line is not one complete JSON object: {line}"
        );
    }
}

/// Cadence was the entire subject of this task's design dispute, and
/// nothing else in the suite measures it. A regression to "short window
/// plus a naive sleep" would keep emitting perfectly valid records at the
/// wrong rate, and every other assertion here would still pass.
#[test]
fn watch_honors_the_requested_interval() {
    vitals_core::skip_without_hardware!();
    use std::io::{BufRead, BufReader};
    use std::process::Stdio;
    use std::time::Instant;

    let mut child = Command::new(env!("CARGO_BIN_EXE_vitals"))
        // -i 2, not -i 1: the interval must exceed core's 1s sample window
        // or the "short window plus sleep" regression is indistinguishable
        // from correct behaviour, and the test proves nothing.
        .args(["watch", "-i", "2", "-n", "3"])
        .stdout(Stdio::piped())
        .spawn()
        .expect("failed to spawn vitals watch");

    let stdout = child.stdout.take().unwrap();
    let mut stamps = Vec::new();
    for line in BufReader::new(stdout).lines() {
        line.unwrap();
        stamps.push(Instant::now());
    }
    child.wait().unwrap();
    assert_eq!(stamps.len(), 3, "expected 3 lines");

    // Skip the first gap: it also carries Sampler::new (~0.6-0.9s).
    let deltas: Vec<u128> = stamps
        .windows(2)
        .skip(1)
        .map(|w| (w[1] - w[0]).as_millis())
        .collect();
    for d in &deltas {
        assert!(
            (1400..2800).contains(&(*d as u64)),
            "asked for 2s between lines, got {d}ms (all: {deltas:?})"
        );
    }
}
