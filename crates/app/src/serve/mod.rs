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
    let mut hosts = request.headers().iter().filter(|h| h.field.equiv("Host"));
    let host = hosts.next().map(|h| h.value.as_str().to_string());
    // Two Host headers is not a shape any real client produces; it is what
    // request smuggling looks like, and picking either one is a guess. A
    // security check resolves ambiguity as no.
    if hosts.next().is_some() {
        return false;
    }

    host_is_loopback(host.as_deref())
}

/// The decision `host_is_local` makes, over the header value alone.
///
/// Split out because a `tiny_http::Request` cannot be constructed in a unit
/// test, and this is the part worth pinning: every case below is a shape a
/// rebinding attack or a real client actually sends.
fn host_is_loopback(host: Option<&str>) -> bool {
    match host {
        // A missing Host is HTTP/1.0 or a hand-rolled client, not a browser.
        None => true,
        Some(h) => match strip_port(h) {
            None => false,
            Some(name) => {
                // A trailing dot is the legal fully-qualified form of the
                // same name.
                let name = name.strip_suffix('.').unwrap_or(name);
                name.eq_ignore_ascii_case("localhost") || name == "127.0.0.1" || name == "::1"
            }
        },
    }
}

/// Host header minus its port, handling bracketed IPv6.
///
/// Splitting on the last `:` is wrong for `[::1]`, which contains colons of
/// its own: it yielded `[:` and refused a legitimate local client. Brackets
/// delimit the address, so read to `]` when present; otherwise only treat a
/// trailing `:digits` as a port, so a hostname containing a stray colon is
/// not silently truncated into something that might compare equal.
fn strip_port(host: &str) -> Option<&str> {
    if let Some(rest) = host.strip_prefix('[') {
        // `[addr]` or `[addr]:port` and nothing else. Reading to the first
        // `]` and discarding the tail would admit `[::1].evil.com`, whose
        // literal prefix is attacker-chosen; the tail has to be checked.
        let (addr, tail) = rest.split_once(']')?;
        return is_port(tail).then_some(addr);
    }
    match host.rsplit_once(':') {
        // Only a trailing `:digits` is a port. A hostname carrying some other
        // colon is left whole rather than truncated into something that might
        // compare equal to a local name.
        Some((name, port)) if is_port(&format!(":{port}")) => Some(name),
        Some(_) => None,
        None => Some(host),
    }
}

/// Empty, or `:` followed by at least one digit and nothing else.
fn is_port(tail: &str) -> bool {
    match tail.strip_prefix(':') {
        None => tail.is_empty(),
        Some(digits) => !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()),
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
/// The port every face of the dashboard agrees on.
///
/// The CLI's `serve` and `dashboard` flags and the tray's menu item all read
/// this. It was a bare `9876` in three places, which is three chances to
/// change one and not the others.
pub const DEFAULT_PORT: u16 = 9876;

/// Guarantee a vitals dashboard is answering on `port`, starting one inside
/// this process if the port is free.
///
/// This is what the tray's "Open Dashboard" item calls. The tray is a
/// long-lived process that already owns a sampler, so a server started here
/// lives and dies with the menu bar item rather than being orphaned as a
/// child process — quitting vitals takes the dashboard with it.
///
/// Returns the address to point a browser at.
pub fn ensure_running(port: u16) -> Result<String, String> {
    let addr = format!("127.0.0.1:{port}");
    let sock: std::net::SocketAddr = addr
        .parse()
        .map_err(|e| format!("bad address {addr}: {e}"))?;
    match probe(&sock) {
        Probe::Vitals => return Ok(addr),
        Probe::Stranger => {
            return Err(format!(
                "Port {port} is already in use by another program, so the \
                 dashboard cannot start. Quit whatever is using it, or run \
                 `vitals serve --port <other>` from a terminal."
            ))
        }
        Probe::Free => {}
    }

    std::thread::spawn(move || {
        // A bind failure here is reported by the readiness loop below timing
        // out, which is the message the user can act on; the raw error would
        // arrive on a thread with nowhere to show it.
        let _ = run(port, false);
    });

    // `run` binds within milliseconds of this thread starting, and answers
    // 503-with-a-schema-version before its first sample lands, so readiness
    // does not wait on the sampler. The generous ceiling is for a machine
    // under the kind of load this app exists to diagnose.
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if probe(&sock) == Probe::Vitals {
            return Ok(addr);
        }
        // A failed probe against an unbound port is a refused connect on
        // loopback — microseconds — so polling tightly here costs nothing
        // and is the difference between a ~10ms and a ~31ms main-thread
        // block for the tray's menu item.
        std::thread::sleep(Duration::from_millis(2));
    }
    Err(format!("the dashboard did not come up on port {port}"))
}

pub fn run_dashboard(port: u16) -> Result<(), String> {
    let addr = format!("127.0.0.1:{port}");
    let sock = addr
        .parse()
        .map_err(|e| format!("bad address {addr}: {e}"))?;
    match probe(&sock) {
        Probe::Vitals => {
            println!("vitals already serving http://{addr}");
            open_in_browser(&addr);
            Ok(())
        }
        Probe::Free => run(port, true),
        Probe::Stranger => Err(format!(
            "port {port} is held by something that is not vitals; \
             stop it, or pick another port with --port"
        )),
    }
}

/// What is on the port.
#[derive(Debug, PartialEq, Eq)]
enum Probe {
    /// Nothing is listening; we can bind it ourselves.
    Free,
    /// A vitals server, which the browser can be pointed at.
    Vitals,
    /// Something is listening, but it did not answer like vitals.
    Stranger,
}

/// Ask whoever holds `addr` whether they are vitals.
///
/// A successful connect only proves *something* is there. Opening a browser
/// at an unidentified local service would hand it the user's attention under
/// the URL they associate with this app — and whatever session cookies that
/// origin already holds. One request settles it: only vitals answers
/// `/api/snapshot` with a schema version (a warming-up vitals answers 503
/// with one too, which is still an identification).
fn probe(addr: &std::net::SocketAddr) -> Probe {
    use std::io::{Read, Write};
    let timeout = std::time::Duration::from_millis(300);
    let Ok(mut s) = std::net::TcpStream::connect_timeout(addr, timeout) else {
        return Probe::Free;
    };
    let _ = s.set_read_timeout(Some(timeout));
    let _ = s.set_write_timeout(Some(timeout));
    let req = b"GET /api/snapshot HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n";
    if s.write_all(req).is_err() {
        return Probe::Stranger;
    }
    // Bounded read: an unknown listener must not be able to stream into this
    // buffer indefinitely, and the marker appears in the first few bytes of
    // the body either way.
    let mut buf = [0u8; 2048];
    let mut n = 0;
    while n < buf.len() {
        match s.read(&mut buf[n..]) {
            Ok(0) | Err(_) => break,
            Ok(k) => n += k,
        }
    }
    if String::from_utf8_lossy(&buf[..n]).contains("\"schema_version\"") {
        Probe::Vitals
    } else {
        Probe::Stranger
    }
}

pub(crate) fn open_in_browser(addr: &str) {
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

    #[test]
    fn only_loopback_names_are_accepted() {
        // Left column is the Host header verbatim; right is whether the
        // request is served. The refusals are the rebinding shapes: a name
        // the attacker controls that merely contains, extends or prefixes a
        // loopback name. Each was checked against a running server.
        let cases = [
            // real local clients
            ("localhost", true),
            ("localhost:8830", true),
            ("LOCALHOST:8830", true),
            ("127.0.0.1", true),
            ("127.0.0.1:8830", true),
            ("[::1]", true),
            ("[::1]:8830", true),
            // a trailing dot is the same name, fully qualified
            ("localhost.", true),
            ("localhost.:9", true),
            // rebinding
            ("evil.com", false),
            ("evil.com:8830", false),
            ("localhost.evil.com", false),
            ("localhost.evil.com.", false),
            ("127.0.0.1.evil.com", false),
            ("xlocalhost", false),
            ("localhosts", false),
            // a bracketed literal must END at the bracket, or the tail is
            // the attacker's: reading to the first `]` admitted this one
            ("[::1].evil.com", false),
            ("[::1]x", false),
            ("[::1]:80x", false),
            ("[::1", false),
            // a colon that is not a port must not truncate the name
            ("localhost:evil", false),
            // IPv6 without brackets is malformed; no real client sends it
            ("::1", false),
        ];
        for (host, want) in cases {
            assert_eq!(
                host_is_loopback(Some(host)),
                want,
                "Host: {host:?} should {} be served",
                if want { "" } else { "not" }
            );
        }
        // HTTP/1.0 and hand-rolled clients send no Host at all.
        assert!(host_is_loopback(None));
    }

    /// Answer one connection with `reply`, then stop. Returns the address.
    fn one_shot_listener(reply: &'static [u8]) -> std::net::SocketAddr {
        use std::io::{Read, Write};
        let l = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = l.local_addr().expect("addr");
        std::thread::spawn(move || {
            if let Ok((mut c, _)) = l.accept() {
                let mut scratch = [0u8; 1024];
                let _ = c.read(&mut scratch);
                let _ = c.write_all(reply);
            }
        });
        addr
    }

    #[test]
    fn an_unheld_port_probes_as_free() {
        // Bind to get a port the OS says is free, then drop it. A tiny race
        // window, but nothing else in the test suite binds to a fixed port.
        let addr = std::net::TcpListener::bind("127.0.0.1:0")
            .expect("bind")
            .local_addr()
            .expect("addr");
        assert_eq!(probe(&addr), Probe::Free);
    }

    #[test]
    fn a_listener_that_answers_like_vitals_is_recognised() {
        let addr = one_shot_listener(
            b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\r\n{\"schema_version\":1}",
        );
        assert_eq!(probe(&addr), Probe::Vitals);
    }

    #[test]
    fn some_other_service_on_the_port_is_not_mistaken_for_vitals() {
        // The whole point: a successful connect is not an identification.
        // Pointing a browser at this would hand an unrelated local service
        // the URL the user trusts as their dashboard.
        let addr = one_shot_listener(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhello");
        assert_eq!(probe(&addr), Probe::Stranger);
    }

    #[test]
    fn ensure_running_starts_a_server_and_returns_fast() {
        // The tray calls this on the AppKit main thread, so how long it
        // blocks is a UI property, not just a performance one: anything
        // approaching a frame budget shows up as the menu stuttering shut.
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .expect("bind")
            .local_addr()
            .expect("addr")
            .port();

        let started = std::time::Instant::now();
        let addr = ensure_running(port).expect("a free port must yield a server");
        let took = started.elapsed();
        assert_eq!(addr, format!("127.0.0.1:{port}"));

        // Measured at ~4ms; 100ms is loose enough not to flake on a loaded
        // machine and tight enough to catch the readiness loop regressing
        // back into a coarse sleep.
        assert!(
            took < std::time::Duration::from_millis(100),
            "ensure_running blocked the main thread for {took:?}"
        );

        // Idempotent: a second call finds the server already up and must not
        // start another or fail on the bind.
        let again = ensure_running(port).expect("second call must reuse the running server");
        assert_eq!(again, addr);
    }

    #[test]
    fn ensure_running_refuses_a_port_held_by_something_else() {
        // The failure the user can actually hit, and the one that has to
        // produce a message rather than a silent no-op.
        let addr = one_shot_listener(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhello");
        let err = ensure_running(addr.port()).expect_err("a stranger must not be served");
        assert!(
            err.contains("already in use"),
            "the message must say what is wrong: {err}"
        );
    }

    #[test]
    fn a_listener_that_accepts_and_says_nothing_is_not_vitals() {
        // Silence must resolve to Stranger, not hang and not pass. The read
        // timeout is what bounds this.
        let l = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = l.local_addr().expect("addr");
        let started = std::time::Instant::now();
        assert_eq!(probe(&addr), Probe::Stranger);
        assert!(
            started.elapsed() < std::time::Duration::from_secs(2),
            "probe must not block on a silent listener"
        );
        drop(l);
    }
}
