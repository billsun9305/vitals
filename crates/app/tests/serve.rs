use std::process::{Child, Command, Stdio};

struct Server(Child);
impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
    }
}

fn start(port: u16) -> Server {
    let child = Command::new(env!("CARGO_BIN_EXE_vitals"))
        .args(["serve", "--port", &port.to_string(), "--no-open"])
        .stdout(Stdio::null())
        .spawn()
        .expect("failed to start vitals serve");
    // Give the listener time to bind and the first sample to land.
    //
    // Measured on an M1 Pro debug build: process startup plus
    // `SamplerSession::new()` (~0.5s) plus the first `get_metrics` call
    // (~0.2s at the server's 200ms cadence) plus up to one 250ms cache-poll
    // tick already totals close to 1s before any HTTP overhead, so a 1.5s
    // budget flakes under load. 3s leaves comfortable headroom.
    std::thread::sleep(std::time::Duration::from_millis(3_000));
    Server(child)
}

fn get(port: u16, path: &str) -> (u16, String) {
    let url = format!("http://127.0.0.1:{port}{path}");
    let out = Command::new("curl")
        .args(["-s", "-o", "/dev/stdout", "-w", "\n%{http_code}", &url])
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    let (body, code) = text.rsplit_once('\n').unwrap();
    (code.trim().parse().unwrap(), body.to_string())
}

#[test]
fn api_snapshot_returns_the_same_schema_as_the_cli() {
    let _s = start(9877);
    let (code, body) = get(9877, "/api/snapshot");
    assert_eq!(code, 200);
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["schema_version"], 1);
    assert!(v["cpu_pct"].is_number());
}

#[test]
fn unknown_paths_are_404_not_a_panic() {
    let _s = start(9878);
    let (code, _) = get(9878, "/nope");
    assert_eq!(code, 404);
}

#[test]
fn routing_is_a_pure_function() {
    use vitals::serve::{route, Route};
    assert_eq!(route("/api/snapshot"), Route::Snapshot);
    assert_eq!(route("/api/top"), Route::Top);
    assert_eq!(route("/api/pressure"), Route::Pressure);
    assert_eq!(route("/"), Route::Index);
    assert_eq!(
        route("/assets/app.js"),
        Route::Asset("assets/app.js".into())
    );
    assert_eq!(route("/../etc/passwd"), Route::NotFound);
}
