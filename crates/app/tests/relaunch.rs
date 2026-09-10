//! The relaunch helper as a subprocess. Neither test launches anything:
//! one names a bundle that does not exist, the other a directory that is
//! not an app, so LaunchServices answers with an error both times. The
//! second is the only automated proof that the completion handler fires
//! in a process that never runs `NSApplication`.

use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

fn relaunch(parent: u32, app: &str) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_vitals"))
        .args(["relaunch", "--parent", &parent.to_string(), "--app", app])
        .output()
        .expect("run vitals relaunch")
}

/// Like `relaunch`, but returns the still-running child instead of blocking
/// on it, so the caller can interleave waiting on the parent and the
/// helper to prove an ordering between the two.
fn spawn_relaunch(parent: u32, app: &str) -> std::process::Child {
    Command::new(env!("CARGO_BIN_EXE_vitals"))
        .args(["relaunch", "--parent", &parent.to_string(), "--app", app])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn vitals relaunch")
}

#[test]
fn the_helper_waits_for_its_parent_before_doing_anything() {
    // The parent's exit and the helper's return have to be observed on two
    // separate threads for `helper_returned >= parent_exited` to prove
    // anything: `parent.wait()` runs on its own thread concurrently with
    // this thread blocking on `helper.wait_with_output()`, so the two
    // waits race for real instead of running one after the other in
    // program order, where the ordering would hold trivially no matter
    // what the helper does. A 2s parent also gives `took` real separation
    // from process-start-plus-dyld noise: a helper that never called
    // `wait_for_exit` would return in well under a second, so
    // `took >= 1500ms` alone already catches a regression that skips the
    // wait, and the cross-thread ordering assertion catches one that waits
    // on the wrong thing.
    let mut parent = Command::new("sleep").arg("2").spawn().unwrap();
    let started = Instant::now();
    let helper = spawn_relaunch(parent.id(), "/nonexistent/Vitals.app");

    let parent_thread = thread::spawn(move || {
        parent.wait().expect("wait for parent");
        Instant::now()
    });

    let out = helper.wait_with_output().expect("wait for helper");
    let helper_returned = Instant::now();
    let took = started.elapsed();

    let parent_exited = parent_thread.join().expect("parent thread panicked");

    assert!(
        helper_returned >= parent_exited,
        "helper returned before the parent was observed to exit"
    );
    assert!(
        took >= Duration::from_millis(1500),
        "returned after {took:?}, too fast to have waited for a 2s parent"
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
