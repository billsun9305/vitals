//! The relaunch helper as a subprocess. Neither test launches anything:
//! one names a bundle that does not exist, the other a directory that is
//! not an app, so LaunchServices answers with an error both times. The
//! second is the only automated proof that the completion handler fires
//! in a process that never runs `NSApplication`.

use std::process::Command;
use std::time::{Duration, Instant};

fn relaunch(parent: u32, app: &str) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_vitals"))
        .args(["relaunch", "--parent", &parent.to_string(), "--app", app])
        .output()
        .expect("run vitals relaunch")
}

#[test]
fn the_helper_waits_for_its_parent_before_doing_anything() {
    let mut parent = Command::new("sleep").arg("0.5").spawn().unwrap();
    let started = Instant::now();
    let out = relaunch(parent.id(), "/nonexistent/Vitals.app");
    let took = started.elapsed();
    let _ = parent.wait();

    assert!(
        took >= Duration::from_millis(400),
        "returned after {took:?}, before the parent exited"
    );
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("does not exist"), "{stderr}");
}

#[test]
fn a_parent_that_is_already_gone_is_not_waited_for() {
    let mut parent = Command::new("true").spawn().unwrap();
    let pid = parent.id();
    let _ = parent.wait(); // reaped: the pid is gone before the helper looks
    let started = Instant::now();
    let out = relaunch(pid, "/nonexistent/Vitals.app");
    assert!(started.elapsed() < Duration::from_secs(5));
    assert!(!out.status.success());
}

#[test]
fn a_path_that_is_not_an_app_is_refused_by_launch_services() {
    let dir = tempfile::tempdir().unwrap();
    let mut parent = Command::new("sleep").arg("0.2").spawn().unwrap();
    let out = relaunch(parent.id(), &dir.path().to_string_lossy());
    let _ = parent.wait();

    assert!(!out.status.success(), "opening a plain directory must fail");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.starts_with("vitals: "), "{stderr}");
    assert!(
        stderr.len() > "vitals: ".len(),
        "LaunchServices' reason should be in the message: {stderr}"
    );
}
