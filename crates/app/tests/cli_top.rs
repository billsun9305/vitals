use std::process::Command;

fn run(args: &[&str]) -> serde_json::Value {
    let out = Command::new(env!("CARGO_BIN_EXE_vitals")).args(args).output().unwrap();
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    serde_json::from_slice(&out.stdout).unwrap()
}

#[test]
fn top_returns_both_dimensions_in_one_call() {
    let v = run(&["top", "-n", "3"]);
    assert_eq!(v["schema_version"], 1);
    let by_cpu = v["by_cpu"].as_array().unwrap();
    let by_mem = v["by_mem"].as_array().unwrap();
    assert_eq!(by_cpu.len(), 3);
    assert_eq!(by_mem.len(), 3);

    for row in by_cpu.iter().chain(by_mem) {
        assert!(row["pid"].is_u64());
        assert!(row["name"].is_string());
        assert!(row["cpu_pct"].is_number());
        assert!(row["mem_mb"].is_u64());
        assert!(row["threads"].is_u64());
    }
}

#[test]
fn top_is_sorted_descending() {
    let v = run(&["top", "-n", "5"]);
    let mem: Vec<u64> = v["by_mem"].as_array().unwrap()
        .iter().map(|r| r["mem_mb"].as_u64().unwrap()).collect();
    assert!(mem.windows(2).all(|w| w[0] >= w[1]), "not descending: {mem:?}");
}

#[test]
fn top_default_n_is_five() {
    let v = run(&["top"]);
    assert_eq!(v["by_cpu"].as_array().unwrap().len(), 5);
}

#[test]
fn by_cpu_is_sorted_descending() {
    // by_mem's ordering is asserted separately; without this, flipping only
    // the CPU comparator in procs::rank would pass every other test, since
    // the rest check lengths and field types but never cpu_pct's order.
    let v = run(&["top", "-n", "5"]);
    let cpu: Vec<f64> = v["by_cpu"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["cpu_pct"].as_f64().unwrap())
        .collect();
    assert!(
        cpu.windows(2).all(|w| w[0] >= w[1]),
        "by_cpu is not descending: {cpu:?}"
    );
    // Descending alone is not enough: a comparator flipped to ascending
    // returns the five idlest processes, all at 0.0%, which satisfies
    // "descending" vacuously. The busiest process on a running Mac is never
    // at zero, so this is what actually pins the direction.
    assert!(
        cpu[0] > 0.0,
        "busiest process reported 0% CPU — ranking is inverted: {cpu:?}"
    );
}
