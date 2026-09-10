//! One update check, and the state the tray keeps about updates.
//!
//! `check_now` blocks and belongs on a worker thread; `spawn_check` puts
//! it there and, when it is done, leaves the outcome in a slot the main
//! thread reads and then calls `notify`, which the tray uses to post the
//! notification its controller observes. `UpdateState::apply` is the pure
//! decision about what an outcome means, and is unit-tested.

use std::sync::{Arc, Mutex};
use std::time::Instant;

use super::http;
use super::release::{parse_latest, Release, Source, Version};

/// Seconds after the tray starts before the first check.
pub const FIRST_CHECK_S: f64 = 30.0;

/// Seconds between checks after the first: one a day.
pub const CHECK_PERIOD_S: f64 = 24.0 * 60.0 * 60.0;

/// Slack macOS may add to the daily check to coalesce it with other
/// wake-ups. An hour late is fine for a once-a-day question.
pub const CHECK_TOLERANCE_S: f64 = 3600.0;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckOutcome {
    /// The latest release is this version, which is not newer than ours.
    UpToDate(Version),
    Available(Release),
    Failed(String),
}

/// Ask `source` for the latest release and compare it with `current`.
pub fn check_now(source: &Source, current: Version) -> CheckOutcome {
    let json = match http::fetch(&source.api_latest) {
        Ok(body) => body,
        Err(e) => return CheckOutcome::Failed(e.to_string()),
    };
    match parse_latest(&json, source) {
        Ok(release) if release.version > current => CheckOutcome::Available(release),
        Ok(release) => CheckOutcome::UpToDate(release.version),
        Err(e) => CheckOutcome::Failed(e.to_string()),
    }
}

/// What the tray knows about updates. Lives in the controller's ivars and
/// is touched only on the main thread.
#[derive(Debug, Default)]
pub struct UpdateState {
    /// A release newer than the running one, if the last successful check
    /// found one.
    pub available: Option<Release>,
    pub checking: bool,
    pub installing: bool,
    pub last_check: Option<(Instant, Result<(), String>)>,
}

impl UpdateState {
    /// Fold a finished check in. A newer "latest" replaces `available`; an
    /// equal or older one clears it; a failure leaves it alone. Every
    /// outcome ends the check.
    pub fn apply(&mut self, outcome: CheckOutcome, now: Instant) {
        self.checking = false;
        match outcome {
            CheckOutcome::Available(release) => {
                self.available = Some(release);
                self.last_check = Some((now, Ok(())));
            }
            CheckOutcome::UpToDate(_) => {
                self.available = None;
                self.last_check = Some((now, Ok(())));
            }
            CheckOutcome::Failed(reason) => {
                self.last_check = Some((now, Err(reason)));
            }
        }
    }
}

/// Where a worker leaves its result for the main thread.
pub type Slot<T> = Arc<Mutex<Option<T>>>;

/// Run `check_now` on a named thread; store `(outcome, manual)` in `slot`,
/// then call `notify`. `manual` rides along so the main thread knows
/// whether to report the outcome in an alert.
pub fn spawn_check(
    source: Source,
    current: Version,
    manual: bool,
    slot: Slot<(CheckOutcome, bool)>,
    notify: impl FnOnce() + Send + 'static,
) {
    std::thread::Builder::new()
        .name("vitals-update-check".into())
        .spawn(move || {
            let outcome = check_now(&source, current);
            *slot.lock().unwrap() = Some((outcome, manual));
            notify();
        })
        .expect("spawning the update check thread");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::update::release::archive_name;
    use std::sync::{Arc, Mutex};

    fn release(version: &str) -> Release {
        let version = Version::parse(version).unwrap();
        Release {
            version,
            notes: String::new(),
            page_url: String::new(),
            archive_url: String::new(),
            sums_url: String::new(),
            archive_name: archive_name(version),
        }
    }

    #[test]
    fn a_newer_release_becomes_available() {
        let mut state = UpdateState {
            checking: true,
            ..Default::default()
        };
        state.apply(CheckOutcome::Available(release("0.2.0")), Instant::now());
        assert_eq!(
            state.available.as_ref().map(|r| r.version.to_string()),
            Some("0.2.0".into())
        );
        assert!(!state.checking);
        assert!(matches!(state.last_check, Some((_, Ok(())))));
    }

    #[test]
    fn a_newer_latest_replaces_an_older_available() {
        let mut state = UpdateState::default();
        state.apply(CheckOutcome::Available(release("0.2.0")), Instant::now());
        state.apply(CheckOutcome::Available(release("0.3.0")), Instant::now());
        assert_eq!(
            state.available.as_ref().map(|r| r.version.to_string()),
            Some("0.3.0".into())
        );
    }

    #[test]
    fn up_to_date_clears_available() {
        let mut state = UpdateState::default();
        state.apply(CheckOutcome::Available(release("0.2.0")), Instant::now());
        state.apply(
            CheckOutcome::UpToDate(Version::parse("0.1.0").unwrap()),
            Instant::now(),
        );
        assert!(state.available.is_none());
        assert!(!state.checking);
    }

    #[test]
    fn a_failure_keeps_available_and_records_the_reason() {
        let mut state = UpdateState {
            checking: true,
            ..Default::default()
        };
        state.apply(CheckOutcome::Available(release("0.2.0")), Instant::now());
        state.apply(CheckOutcome::Failed("offline".into()), Instant::now());
        assert!(
            state.available.is_some(),
            "a failed check must not forget a known update"
        );
        assert!(!state.checking);
        assert!(matches!(&state.last_check, Some((_, Err(r))) if r == "offline"));
    }

    /// A loopback server answering `/releases/latest` with `body` at `status`.
    struct Fixture {
        base: String,
        server: Arc<tiny_http::Server>,
        thread: Option<std::thread::JoinHandle<()>>,
    }

    fn serve_latest(status: u16, body: Vec<u8>) -> Fixture {
        let server = Arc::new(tiny_http::Server::http("127.0.0.1:0").unwrap());
        let port = server.server_addr().to_ip().unwrap().port();
        let s = Arc::clone(&server);
        let thread = std::thread::spawn(move || {
            for request in s.incoming_requests() {
                let _ = request
                    .respond(tiny_http::Response::from_data(body.clone()).with_status_code(status));
            }
        });
        Fixture {
            base: format!("http://127.0.0.1:{port}/"),
            server,
            thread: Some(thread),
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            self.server.unblock();
            if let Some(t) = self.thread.take() {
                let _ = t.join();
            }
        }
    }

    fn latest_json(base: &str, version: &str) -> Vec<u8> {
        let name = archive_name(Version::parse(version).unwrap());
        serde_json::to_vec(&serde_json::json!({
            "tag_name": format!("v{version}"), "draft": false, "prerelease": false,
            "html_url": format!("{base}releases/tag/v{version}"), "body": "notes",
            "assets": [
                {"name": name, "browser_download_url": format!("{base}{name}")},
                {"name": "SHA256SUMS", "browser_download_url": format!("{base}SHA256SUMS")}
            ]
        }))
        .unwrap()
    }

    #[test]
    fn check_now_reports_newer_same_and_broken() {
        let current = Version::parse("0.1.0").unwrap();

        let newer = {
            let server = Arc::new(tiny_http::Server::http("127.0.0.1:0").unwrap());
            let port = server.server_addr().to_ip().unwrap().port();
            let base = format!("http://127.0.0.1:{port}/");
            let body = latest_json(&base, "0.2.0");
            let s = Arc::clone(&server);
            let thread = std::thread::spawn(move || {
                for request in s.incoming_requests() {
                    let _ = request.respond(tiny_http::Response::from_data(body.clone()));
                }
            });
            Fixture {
                base,
                server,
                thread: Some(thread),
            }
        };
        let outcome = check_now(&Source::loopback(&newer.base).unwrap(), current);
        assert!(
            matches!(&outcome, CheckOutcome::Available(r) if r.version.to_string() == "0.2.0"),
            "{outcome:?}"
        );

        let same = {
            let server = Arc::new(tiny_http::Server::http("127.0.0.1:0").unwrap());
            let port = server.server_addr().to_ip().unwrap().port();
            let base = format!("http://127.0.0.1:{port}/");
            let body = latest_json(&base, "0.1.0");
            let s = Arc::clone(&server);
            let thread = std::thread::spawn(move || {
                for request in s.incoming_requests() {
                    let _ = request.respond(tiny_http::Response::from_data(body.clone()));
                }
            });
            Fixture {
                base,
                server,
                thread: Some(thread),
            }
        };
        let outcome = check_now(&Source::loopback(&same.base).unwrap(), current);
        assert!(
            matches!(&outcome, CheckOutcome::UpToDate(v) if v.to_string() == "0.1.0"),
            "{outcome:?}"
        );

        let broken = serve_latest(503, b"nope".to_vec());
        let outcome = check_now(&Source::loopback(&broken.base).unwrap(), current);
        assert!(
            matches!(&outcome, CheckOutcome::Failed(r) if r.contains("503")),
            "{outcome:?}"
        );
    }

    #[test]
    fn spawn_check_fills_the_slot_then_notifies() {
        let fx = serve_latest(404, Vec::new());
        let slot: Slot<(CheckOutcome, bool)> = Arc::new(Mutex::new(None));
        let (tx, rx) = std::sync::mpsc::channel();
        spawn_check(
            Source::loopback(&fx.base).unwrap(),
            Version::parse("0.1.0").unwrap(),
            true,
            Arc::clone(&slot),
            move || {
                let _ = tx.send(());
            },
        );
        rx.recv_timeout(std::time::Duration::from_secs(20))
            .expect("notified");
        let (outcome, manual) = slot
            .lock()
            .unwrap()
            .take()
            .expect("slot filled before notify");
        assert!(manual);
        assert!(matches!(outcome, CheckOutcome::Failed(_)));
    }
}
