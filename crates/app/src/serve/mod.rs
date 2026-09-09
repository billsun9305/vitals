//! Localhost HTTP face. Bound to 127.0.0.1 only, no auth, no `--host` flag
//! to regret later.

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

/// Decode `%XX` percent-escapes in a request path. Hex digits are matched
/// case-insensitively (`%2e` and `%2E` decode the same), so this alone
/// collapses one whole family of traversal-by-encoding tricks. A malformed
/// `%` sequence (missing or non-hex digits, or a `%` with fewer than two
/// bytes left in the string) is passed through untouched rather than
/// rejected outright: it can never *produce* a `..` or `\`, which is the
/// only thing `route()` cares about, so treating it strictly is unnecessary.
fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hi = (bytes[i + 1] as char).to_digit(16);
            let lo = (bytes[i + 2] as char).to_digit(16);
            if let (Some(h), Some(l)) = (hi, lo) {
                out.push(((h * 16) + l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Pure request routing.
///
/// The path is percent-decoded *before* the traversal check, so an encoded
/// form like `%2e%2e` (any hex case) is caught exactly like a literal `..`.
/// Backslash is rejected outright too — Unix path lookups do not treat it as
/// a separator, but nothing here needs to permit it in a legitimate asset
/// path, and rejecting it removes a whole class of "does the underlying
/// matcher treat `\` as `/`" questions before they can matter.
///
/// This check is defence in depth, not the defence: `assets::serve` only
/// ever resolves a path against the fixed set of files embedded at compile
/// time (`include_dir!`), so there is no filesystem to escape to even if a
/// `..` slipped through here. See `assets.rs`.
pub fn route(path: &str) -> Route {
    let path = path.split('?').next().unwrap_or("");
    let decoded = percent_decode(path);
    if decoded.contains("..") || decoded.contains('\\') {
        return Route::NotFound;
    }
    match decoded.as_str() {
        "/" | "/index.html" => Route::Index,
        "/api/snapshot" => Route::Snapshot,
        "/api/top" => Route::Top,
        "/api/pressure" => Route::Pressure,
        p if p.starts_with('/') && p.len() > 1 => {
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
pub(super) fn error_body(e: &str) -> String {
    // `schema_version` on the error too: the contract in docs/agents.md is
    // that EVERY response carries it, and an agent that mistypes an endpoint
    // should get a structured answer rather than something it cannot parse.
    serde_json::json!({
        "schema_version": vitals_core::schema::SCHEMA_VERSION,
        "error": e,
    })
    .to_string()
}

/// Reject requests whose `Host` is not loopback.
///
/// The listener is bound to 127.0.0.1, which stops direct remote connections
/// but not DNS rebinding: a page on an attacker's domain can re-resolve that
/// domain to 127.0.0.1 and have the victim's own browser make the request,
/// from which it would read the full process table — every running
/// application's name — plus the machine model and uptime. The browser sends
/// the attacker's hostname in `Host`; a real local client sends localhost.
/// One comparison closes it.
fn host_is_local(request: &tiny_http::Request) -> bool {
    let host = request
        .headers()
        .iter()
        .find(|h| h.field.equiv("Host"))
        .map(|h| h.value.as_str().to_string());
    match host {
        // A missing Host is HTTP/1.0 or a hand-rolled client, not a browser.
        None => true,
        Some(h) => {
            let name = h.rsplit_once(':').map_or(h.as_str(), |(n, _)| n);
            let name = name.trim_start_matches('[').trim_end_matches(']');
            name.eq_ignore_ascii_case("localhost") || name == "127.0.0.1" || name == "::1"
        }
    }
}

/// Latest snapshot, refreshed by a background sampler while anyone is watching.
struct Cache {
    latest: Mutex<Option<Snapshot>>,
    last_request: Mutex<Instant>,
}

/// Open the dashboard, starting a server only if one is not already there.
///
/// `run` alone cannot do this: it binds first, so a second invocation dies
/// with "Address already in use" — which is exactly the case the `dashboard`
/// verb's help text promises to handle ("start the server if needed"). Probe
/// the port first; if something is already serving, just point the browser at
/// it and return.
pub fn run_dashboard(port: u16) -> Result<(), String> {
    let addr = format!("127.0.0.1:{port}");
    if std::net::TcpStream::connect_timeout(
        &addr
            .parse()
            .map_err(|e| format!("bad address {addr}: {e}"))?,
        std::time::Duration::from_millis(300),
    )
    .is_ok()
    {
        println!("vitals already serving http://{addr}");
        open_in_browser(&addr);
        return Ok(());
    }
    run(port, true)
}

fn open_in_browser(addr: &str) {
    let _ = std::process::Command::new("open")
        .arg(format!("http://{addr}"))
        .status();
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
        open_in_browser(&addr);
    }

    for request in server.incoming_requests() {
        *cache.last_request.lock().unwrap() = Instant::now();
        if !host_is_local(&request) {
            let _ = request.respond(
                Response::from_string(error_body("request rejected: non-local Host header"))
                    .with_header(json_header())
                    .with_status_code(403),
            );
            continue;
        }
        let r = route(request.url());
        let response = match r {
            Route::Snapshot => match cache.latest.lock().unwrap().clone() {
                Some(s) => Response::from_string(serde_json::to_string(&s).unwrap())
                    .with_header(json_header()),
                None => Response::from_string(error_body("warming up"))
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
            // ReadOnly: this endpoint is polled every 5s by the dashboard. Writing
            // the baseline here would reset it on every poll and silently disable
            // swap-trend detection for every reader, the CLI included.
            // Reuse the cached snapshot the background sampler already took.
            // Calling the collecting variant here would stand up a second
            // `macmon::Sampler` on every poll and sample the SoC alongside
            // our own worker.
            Route::Pressure => match cache.latest.lock().unwrap().clone() {
                None => Response::from_string(error_body("warming up"))
                    .with_header(json_header())
                    .with_status_code(503),
                Some(snap) => {
                    match crate::cli::pressure_from_snapshot(snap, crate::cli::Baseline::ReadOnly) {
                        Ok(v) => Response::from_string(serde_json::to_string(&v).unwrap())
                            .with_header(json_header()),
                        Err(e) => Response::from_string(error_body(&e))
                            .with_header(json_header())
                            .with_status_code(500),
                    }
                }
            },
            Route::Index => assets::index(),
            Route::Asset(p) => assets::serve(&p),
            // JSON, not the bare "not found" text this used to send: an agent
            // that mistypes an endpoint got a parse error instead of an answer.
            Route::NotFound => Response::from_string(error_body("not found"))
                .with_header(json_header())
                .with_status_code(404),
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
