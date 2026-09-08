//! Real asset server: the React dashboard's build output, embedded into the
//! binary at compile time.
//!
//! Every byte handed back here comes from `DIST`, a compile-time snapshot of
//! `dashboard/dist` baked in by `include_dir!`. There is no filesystem
//! access at runtime and no path joining against a real directory — `serve`
//! does a single lookup (`DIST.get_file`) against the fixed set of paths the
//! build produced, and anything not in that set is a 404. That is what
//! actually rules out path traversal: even if a `..` or backslash somehow
//! reached this function, there is no `..` to escape *to* — the "filesystem"
//! is a fixed, closed set of keys baked in at build time, and a lookup
//! either hits one of those keys or hits nothing. `route()`'s decode-then-
//! reject check (`crates/app/src/serve/mod.rs`) is defence in depth on top
//! of that, not the thing making this safe.
//!
//! `include_dir!` fails the *build* if `dashboard/dist` doesn't exist at
//! all, which is why the repo commits a placeholder `dashboard/dist/.gitkeep`
//! (see the top-level `.gitignore` and `Makefile`) — a fresh clone can
//! `cargo check`/`cargo build` before anyone has run `make dashboard`. What
//! it does not guarantee is that `index.html` is actually *in* there; until
//! `make dashboard` runs, `DIST` holds nothing but the placeholder, and
//! `index()` reports that plainly instead of serving nothing or panicking.

use include_dir::{include_dir, Dir};
use std::io::Cursor;
use tiny_http::{Header, Response};

static DIST: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/../../dashboard/dist");

fn content_type(path: &str) -> &'static str {
    match path.rsplit('.').next().unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "js" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "svg" => "image/svg+xml",
        "json" => "application/json",
        "ico" => "image/x-icon",
        "png" => "image/png",
        "woff2" => "font/woff2",
        _ => "application/octet-stream",
    }
}

fn respond(path: &str, bytes: &[u8]) -> Response<Cursor<Vec<u8>>> {
    Response::from_data(bytes.to_vec()).with_header(
        Header::from_bytes(&b"Content-Type"[..], content_type(path).as_bytes()).unwrap(),
    )
}

/// Serve the dashboard's `index.html`, or a plain explanatory page if the
/// dashboard hasn't been built into this checkout yet (`dashboard/dist` is
/// present as a directory — the build requires that — but may hold nothing
/// but the committed `.gitkeep` placeholder).
pub fn index() -> Response<Cursor<Vec<u8>>> {
    match DIST.get_file("index.html") {
        Some(f) => respond("index.html", f.contents()),
        None => {
            Response::from_string("dashboard not built; run `make dashboard`").with_status_code(500)
        }
    }
}

/// Serve one embedded asset by its path relative to `dashboard/dist`
/// (already decoded and traversal-checked by `route()`). Anything that
/// isn't an exact key in the embedded set — including every traversal
/// attempt, since none of them can ever spell a key `include_dir!` actually
/// produced — is a 404.
pub fn serve(path: &str) -> Response<Cursor<Vec<u8>>> {
    match DIST.get_file(path) {
        Some(f) => respond(path, f.contents()),
        None => Response::from_string("not found").with_status_code(404),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_type_maps_known_extensions() {
        assert_eq!(content_type("index.html"), "text/html; charset=utf-8");
        assert_eq!(content_type("app.js"), "text/javascript; charset=utf-8");
        assert_eq!(content_type("app.css"), "text/css; charset=utf-8");
        assert_eq!(content_type("icon.svg"), "image/svg+xml");
    }

    #[test]
    fn content_type_falls_back_to_octet_stream_for_unknown_extensions() {
        assert_eq!(content_type("data.bin"), "application/octet-stream");
        assert_eq!(content_type("no-extension"), "application/octet-stream");
    }

    #[test]
    fn unknown_asset_keys_are_404_never_a_filesystem_read() {
        // These are exactly the strings `route()` would hand to `serve()`
        // after decoding, if its own traversal check were ever bypassed.
        // `serve` must still reject them, on its own, by simple absence
        // from the embedded set -- proving the allowlist-by-construction
        // claim in the module doc comment rather than just asserting it.
        for p in [
            "../../../../etc/passwd",
            "..\\..\\etc\\passwd",
            "/etc/passwd",
            "etc/passwd",
        ] {
            let r = serve(p);
            assert_eq!(r.status_code().0, 404, "expected 404 for {p:?}");
        }
    }
}
