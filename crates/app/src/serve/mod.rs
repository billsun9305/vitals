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

/// Pure request routing, including traversal rejection.
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

    // 200ms, not the 1s "menu open" tray cadence: `spawn_sampler`'s worker
    // measures each sample over `interval.min(SAMPLE_WINDOW_MS)`, so a 1s
    // interval makes even the very first sample take a full second to
    // measure — on top of the ~600ms `Sampler::new()` already costs, that
    // leaves a freshly started server unable to answer `/api/snapshot`
    // inside a couple of seconds. 200ms matches the CLI's own
    // `DEFAULT_INTERVAL_MS` and keeps every sample, first included, quick
    // enough for a polling browser tab.
    let cadence = Arc::new(Cadence::new(DEFAULT_INTERVAL_MS));
    let handle = spawn_sampler(Arc::clone(&cadence));
    let bg = Arc::clone(&cache);
    std::thread::spawn(move || loop {
        // Park the sampler when nobody has polled for 10s: a closed tab
        // should cost nothing, which is the whole argument for this face.
        let idle = bg.last_request.lock().unwrap().elapsed() > Duration::from_secs(10);
        if idle {
            cadence.park();
        } else {
            cadence.unpark();
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
                Err(e) => Response::from_string(format!(r#"{{"error":"{e}"}}"#))
                    .with_header(json_header())
                    .with_status_code(500),
            },
            Route::Pressure => match crate::cli::collect_pressure(DEFAULT_INTERVAL_MS) {
                Ok(v) => Response::from_string(serde_json::to_string(&v).unwrap())
                    .with_header(json_header()),
                Err(e) => Response::from_string(format!(r#"{{"error":"{e}"}}"#))
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
