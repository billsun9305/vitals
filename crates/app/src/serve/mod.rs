//! Localhost HTTP face. Bound to 127.0.0.1 only, no auth, no `--host` flag
//! to regret later.

use crate::cli::DEFAULT_INTERVAL_MS;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tiny_http::{Header, Response, Server};
use vitals_core::cadence::Cadence;
use vitals_core::host::{load_avg, uptime_s};
use vitals_core::sample::spawn_sampler;
use vitals_core::schema::{build_snapshot, now_rfc3339, Snapshot, SnapshotInputs};
use vitals_core::sysctl::mem_pressure_level;
use vitals_core::thermal::thermal_state;

#[derive(Debug, PartialEq, Eq)]
pub enum Route {
    Index,
    Snapshot,
    Top,
    Pressure,
    Asset(String),
    NotFound,
}

/// Pure request routing. Rejects any asset path containing a literal `..`
/// segment, which blocks naive traversal attempts, but the check runs on
/// the raw path with no percent-decoding first — an encoded form such as
/// `%2e%2e` is not caught here. No live exploit today: `assets::serve` is
/// still a stub that always 404s regardless of path (see `assets.rs`). The
/// real asset server (Task 19) must decode the path before checking it, or
/// otherwise close this gap.
pub fn route(path: &str) -> Route {
    let path = path.split('?').next().unwrap_or("");
    match path {
        "/" | "/index.html" => Route::Index,
        "/api/snapshot" => Route::Snapshot,
        "/api/top" => Route::Top,
        "/api/pressure" => Route::Pressure,
        p if p.starts_with('/') && !p.contains("..") && p.len() > 1 => {
            Route::Asset(p.trim_start_matches('/').to_string())
        }
        _ => Route::NotFound,
    }
}

fn json_header() -> Header {
    Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).unwrap()
}

/// Build a `{"error": "..."}` body with the message properly JSON-escaped,
/// so an error string containing a `"` or `\` still produces valid JSON —
/// every agent-facing response on this server is contractually valid JSON.
fn error_body(e: &str) -> String {
    serde_json::json!({ "error": e }).to_string()
}

/// Latest snapshot, refreshed by a background sampler while anyone is watching.
struct Cache {
    latest: Mutex<Option<Snapshot>>,
    last_request: Mutex<Instant>,
}

pub fn run(port: u16, open_browser: bool) -> Result<(), String> {
    let cache = Arc::new(Cache {
        latest: Mutex::new(None),
        last_request: Mutex::new(Instant::now()),
    });

    // Per the brief: 1s between samples, matching the tray's own "menu
    // open" cadence. The first sample is unavoidably slow (Sampler::new()
    // plus one measurement window), and the test harness budgets startup
    // time for that instead of forcing a faster steady-state cadence here.
    // The surplus samples a tighter cadence would produce are discarded
    // anyway: the cache thread below polls at 250ms and keeps only the
    // newest one.
    let cadence = Arc::new(Cadence::new(1_000));
    let handle = spawn_sampler(Arc::clone(&cadence));
    let bg = Arc::clone(&cache);
    std::thread::spawn(move || loop {
        // Park the sampler when nobody has polled for 10s: a closed tab
        // should cost nothing, which is the whole argument for this face.
        // Both `park()` and `unpark()` take a mutex and notify a condvar
        // unconditionally, so only call the one that actually changes
        // state — otherwise an idle, already-parked server would wake the
        // sampler thread on every 250ms tick forever.
        let idle = bg.last_request.lock().unwrap().elapsed() > Duration::from_secs(10);
        if idle != cadence.is_parked() {
            if idle {
                cadence.park();
            } else {
                cadence.unpark();
            }
        }

        if let Some(Ok(sample)) = handle.try_recv() {
            let snap = build_snapshot(SnapshotInputs {
                metrics: &sample.metrics,
                host: sample.host.clone(),
                sample_ms: sample.sample_ms,
                sampled_at: now_rfc3339(),
                mem_pressure: mem_pressure_level(),
                thermal_state: thermal_state(),
                load_avg: load_avg(),
                uptime_s: uptime_s(),
            });
            *bg.latest.lock().unwrap() = Some(snap);
        }
        std::thread::sleep(Duration::from_millis(250));
    });

    let addr = format!("127.0.0.1:{port}");
    let server = Server::http(&addr).map_err(|e| format!("cannot bind {addr}: {e}"))?;
    println!("vitals serving http://{addr}");
    if open_browser {
        let _ = std::process::Command::new("open")
            .arg(format!("http://{addr}"))
            .status();
    }

    for request in server.incoming_requests() {
        *cache.last_request.lock().unwrap() = Instant::now();
        let r = route(request.url());
        let response = match r {
            Route::Snapshot => match cache.latest.lock().unwrap().clone() {
                Some(s) => Response::from_string(serde_json::to_string(&s).unwrap())
                    .with_header(json_header()),
                None => Response::from_string(r#"{"error":"warming up"}"#)
                    .with_header(json_header())
                    .with_status_code(503),
            },
            Route::Top => match crate::cli::collect_top(10) {
                Ok(t) => Response::from_string(serde_json::to_string(&t).unwrap())
                    .with_header(json_header()),
                Err(e) => Response::from_string(error_body(&e))
                    .with_header(json_header())
                    .with_status_code(500),
            },
            Route::Pressure => match crate::cli::collect_pressure(DEFAULT_INTERVAL_MS) {
                Ok(v) => Response::from_string(serde_json::to_string(&v).unwrap())
                    .with_header(json_header()),
                Err(e) => Response::from_string(error_body(&e))
                    .with_header(json_header())
                    .with_status_code(500),
            },
            Route::Index => assets::index(),
            Route::Asset(p) => assets::serve(&p),
            Route::NotFound => Response::from_string("not found").with_status_code(404),
        };
        let _ = request.respond(response);
    }
    Ok(())
}

mod assets;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_body_escapes_quotes_and_backslashes_into_valid_json() {
        let body = error_body(r#"bad path: "..\weird""#);
        let v: serde_json::Value =
            serde_json::from_str(&body).expect("error body must be valid JSON");
        assert_eq!(v["error"], r#"bad path: "..\weird""#);
    }
}
