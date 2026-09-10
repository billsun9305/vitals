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
    let server = Server(child);
    wait_until_ready(port);
    server
}

/// Block until the server answers `/api/snapshot` with a 200, or give up.
///
/// A fixed sleep cannot be right here. Startup is process spawn, plus
/// `SamplerSession::new()` (~0.5s), plus one full measurement window — and
/// that window is `interval.min(SAMPLE_WINDOW_MS)`, so it tracks the
/// server's cadence rather than being a constant. Measured on a debug build
/// at the 1s cadence, the first 200 lands around 2.15s; a 3s sleep leaves
/// under a second of margin, which is the kind of number that passes on an
/// idle laptop and flakes on a loaded CI box. Polling removes the guess
/// entirely: the fast case stops as soon as the sample is there, and the
/// ceiling is generous because it is only ever paid by a real failure.
fn wait_until_ready(port: u16) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    loop {
        if get(port, "/api/snapshot").0 == 200 {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "server on port {port} never served /api/snapshot within 20s"
        );
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}

/// `--path-as-is` stops curl from squashing `/../` sequences itself before
/// the request ever leaves the client, which would otherwise make it
/// impossible to test how the server handles a literal `..` on the wire.
fn get(port: u16, path: &str) -> (u16, String) {
    let url = format!("http://127.0.0.1:{port}{path}");
    let out = Command::new("curl")
        .args([
            "-s",
            "--path-as-is",
            "-o",
            "/dev/stdout",
            "-w",
            "\n%{http_code}",
            &url,
        ])
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    let (body, code) = text.rsplit_once('\n').unwrap();
    (code.trim().parse().unwrap(), body.to_string())
}

/// Pull the first built-asset reference out of the served `index.html`, e.g.
/// `src="./assets/index-abc123.js"` -> `/assets/index-abc123.js`. Vite hashes
/// asset filenames, so this reads the real name out of the page rather than
/// hardcoding one.
fn extract_first_asset_href(html: &str) -> Option<String> {
    let start = html.find("assets/")?;
    let end = html[start..].find('"')? + start;
    Some(format!("/{}", &html[start..end]))
}

#[test]
fn api_snapshot_returns_the_same_schema_as_the_cli() {
    vitals_core::skip_without_hardware!();
    let _s = start(9877);
    let (code, body) = get(9877, "/api/snapshot");
    assert_eq!(code, 200);
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["schema_version"], 1);
    assert!(v["cpu_pct"].is_number());
}

#[test]
fn unknown_paths_are_404_not_a_panic() {
    vitals_core::skip_without_hardware!();
    let _s = start(9878);
    let (code, _) = get(9878, "/nope");
    assert_eq!(code, 404);
}

#[test]
fn routing_is_a_pure_function() {
    vitals_core::skip_without_hardware!();
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

/// `route()`'s traversal check must run on the *decoded* path. A raw
/// substring check for `..` (the stub-era behavior) never sees the `..` in
/// a percent-encoded or mixed-case-hex request, so it lets it through as a
/// normal `Route::Asset`. This is exactly the gap the doc comment on
/// `route()` used to warn about — harmless only as long as `assets::serve`
/// stubbed out every path, which Task 19 replaces with a real file server.
#[test]
fn route_rejects_percent_encoded_and_backslash_traversal() {
    vitals_core::skip_without_hardware!();
    use vitals::serve::{route, Route};
    assert_eq!(
        route("/%2e%2e/%2e%2e/etc/passwd"),
        Route::NotFound,
        "lowercase percent-encoded .. must be caught after decoding"
    );
    assert_eq!(
        route("/%2E%2E/%2E%2E/etc/passwd"),
        Route::NotFound,
        "uppercase-hex percent-encoding must decode the same as lowercase"
    );
    assert_eq!(
        route("/%2e%2E/%2E%2e/etc/passwd"),
        Route::NotFound,
        "mixed-case hex digits must still decode to .."
    );
    assert_eq!(
        route("/..%2f..%2fetc/passwd"),
        Route::NotFound,
        "partially encoded traversal (literal .. + encoded slash)"
    );
    assert_eq!(
        route("/../../etc/passwd"),
        Route::NotFound,
        "plain literal traversal must still be rejected"
    );
    assert_eq!(
        route("/..%5c..%5cetc%5cpasswd"),
        Route::NotFound,
        "encoded backslash must decode and be rejected too"
    );
    assert_eq!(
        route("/assets\\..\\..\\etc\\passwd"),
        Route::NotFound,
        "raw backslash segments must be rejected"
    );
    // A legitimate nested asset path must still route normally.
    assert_eq!(
        route("/assets/index-abc123.js"),
        Route::Asset("assets/index-abc123.js".into())
    );
}

/// End-to-end: hit a live server with the same traversal attempts over real
/// HTTP and confirm neither a 200 nor any file content comes back. This is
/// the check that actually matters once `assets::serve` stops being a stub
/// that 404s unconditionally.
#[test]
fn path_traversal_over_http_is_rejected_and_leaks_nothing() {
    vitals_core::skip_without_hardware!();
    let _s = start(9881);
    for p in [
        "/%2e%2e/%2e%2e/etc/passwd",
        "/../../etc/passwd",
        "/..%2f..%2fetc/passwd",
    ] {
        let (code, body) = get(9881, p);
        assert_eq!(code, 404, "expected 404 for {p}, got {code}");
        assert!(
            !body.contains("root:"),
            "response for {p} looks like it leaked /etc/passwd: {body}"
        );
    }
}

/// The positive case that justifies the negative ones above: a real,
/// embedded asset referenced from the served `index.html` must still come
/// back with a 200. A traversal fix that also breaks legitimate nested
/// asset paths (e.g. by over-eagerly rejecting any path with a `.` in it)
/// would pass every test above and still be useless.
#[test]
fn a_real_embedded_asset_is_served() {
    vitals_core::skip_without_hardware!();
    let _s = start(9882);
    let (index_code, index_body) = get(9882, "/");

    // `include_dir!` bakes in whatever `dashboard/dist` held at compile
    // time, and a fresh clone holds only the committed `.gitkeep` until
    // someone runs `make dashboard`. Asserting 200 unconditionally would
    // make plain `cargo test` — the command every newcomer and every
    // unconfigured CI runs first — fail on a correct checkout.
    //
    // Skipping outright would be worse: a test that silently passes when
    // it did nothing is how a real regression hides. So assert on BOTH
    // states instead. Un-built is a specific, deliberate 500 from
    // `assets::index`, and that contract is worth pinning too — it is what
    // a user who forgot `make dashboard` actually sees.
    if index_code == 500 {
        assert!(
            index_body.contains("dashboard not built"),
            "an un-built dashboard must say so plainly, got: {index_body}"
        );
        eprintln!(
            "note: dashboard not built into this binary; asserted the \
             un-built contract instead. `make test` builds it first and \
             exercises the served-asset path."
        );
        return;
    }

    assert_eq!(
        index_code, 200,
        "index must be either 200 or a 500 saying it is not built"
    );
    let asset_path = extract_first_asset_href(&index_body)
        .expect("a built index.html must reference at least one asset under assets/");
    let (code, _) = get(9882, &asset_path);
    assert_eq!(code, 200, "expected referenced asset {asset_path} to serve");
}

/// Every response is parseable JSON carrying `schema_version` — errors too.
///
/// docs/agents.md promises this without qualification, and an agent that
/// mistypes an endpoint is exactly the caller that most needs a structured
/// answer rather than a bare `not found` it cannot parse. Note the unknown
/// path is served by `assets::serve`, not `Route::NotFound`: anything under
/// `/` is routed as an asset request first, so that is the common 404.
#[test]
fn every_error_response_is_json_with_a_schema_version() {
    vitals_core::skip_without_hardware!();
    let _s = start(9883);
    for path in [
        "/nope",
        "/deeply/nested/missing.js",
        "/favicon.does-not-exist",
    ] {
        let (code, body) = get(9883, path);
        assert_eq!(code, 404, "expected 404 for {path}");
        let v: serde_json::Value = serde_json::from_str(&body)
            .unwrap_or_else(|e| panic!("body for {path} is not JSON ({e}): {body}"));
        assert_eq!(v["schema_version"], 1, "missing schema_version for {path}");
        assert!(v["error"].is_string(), "missing error string for {path}");
    }
}

/// A non-loopback `Host` is refused, because binding 127.0.0.1 does not stop
/// DNS rebinding: an attacker's page can re-resolve its own domain to
/// 127.0.0.1 and have the victim's browser make the request, then read the
/// full process table. The browser sends the attacker's hostname in `Host`.
#[test]
fn a_non_local_host_header_is_rejected() {
    vitals_core::skip_without_hardware!();
    let _s = start(9884);

    let out = std::process::Command::new("curl")
        .args([
            "-s",
            "-o",
            "/dev/stdout",
            "-w",
            "\n%{http_code}",
            "-H",
            "Host: evil.example.com",
            "http://127.0.0.1:9884/api/snapshot",
        ])
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    let (body, code) = text.rsplit_once('\n').unwrap();
    assert_eq!(
        code.trim(),
        "403",
        "a rebound Host must be refused, got {code}"
    );
    assert!(
        !body.contains("\"cpu_pct\""),
        "the refused response leaked snapshot data: {body}"
    );

    // A real local client must still work, port suffix and all.
    let out = std::process::Command::new("curl")
        .args([
            "-s",
            "-o",
            "/dev/stdout",
            "-w",
            "\n%{http_code}",
            "-H",
            "Host: localhost:9884",
            "http://127.0.0.1:9884/api/snapshot",
        ])
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    let (_, code) = text.rsplit_once('\n').unwrap();
    assert_eq!(code.trim(), "200", "localhost must still be served");
}

/// Two `Host` headers must be refused rather than resolved to either one.
///
/// curl cannot send a duplicate header cleanly, so this speaks HTTP directly.
/// No browser produces this shape; a smuggling proxy does, and picking the
/// first — as `.find()` did — means the header the *server* validates is not
/// necessarily the one an intermediary routed on.
#[test]
fn duplicate_host_headers_are_refused() {
    vitals_core::skip_without_hardware!();
    use std::io::{Read, Write};

    let _s = start(9886);

    fn send(raw: &str) -> String {
        let mut c = std::net::TcpStream::connect("127.0.0.1:9886").expect("connect");
        c.set_read_timeout(Some(std::time::Duration::from_secs(5)))
            .expect("timeout");
        c.write_all(raw.as_bytes()).expect("write");
        let mut out = String::new();
        let _ = c.read_to_string(&mut out);
        out
    }

    for order in [
        "Host: localhost\r\nHost: evil.example.com",
        "Host: evil.example.com\r\nHost: localhost",
        "Host: localhost\r\nHost: localhost",
    ] {
        let resp = send(&format!(
            "GET /api/snapshot HTTP/1.1\r\n{order}\r\nConnection: close\r\n\r\n"
        ));
        assert!(
            resp.starts_with("HTTP/1.1 403"),
            "two Host headers must be refused ({order:?}), got: {}",
            resp.lines().next().unwrap_or("")
        );
        assert!(
            !resp.contains("\"cpu_pct\""),
            "the refused response leaked snapshot data"
        );
    }

    // One Host header, same wire path: still served.
    let resp = send("GET /api/snapshot HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n");
    assert!(
        resp.starts_with("HTTP/1.1 200"),
        "a single Host must still be served, got: {}",
        resp.lines().next().unwrap_or("")
    );
}
