//! HTTP over `NSURLSession`: one function that fetches a small body into
//! memory and one that downloads to a file.
//!
//! Both block the calling thread — they are only ever called from the
//! checker's and the installer's worker threads — and both build a fresh
//! ephemeral session per call and invalidate it afterwards, so nothing
//! lingers between checks (the idle budget in `docs/budget.md`). Two
//! timeouts bound that: `timeoutIntervalForRequest` caps idle time (10 s
//! between bytes) and `timeoutIntervalForResource` caps the whole transfer
//! (10 min), because the idle timer alone resets on every byte and a
//! trickling connection could otherwise hold the session, the worker
//! thread and its socket open forever — nothing of a request may outlive
//! it.
//!
//! The completion block runs on a queue of the session's choosing, never
//! on the caller's thread, so it must not hand Objective-C objects back
//! across: it copies what it needs into plain Rust values and sends them
//! down an `mpsc` channel, the same rule `tray/child.rs` follows for the
//! launch completion handler.

use std::path::{Path, PathBuf};
use std::ptr::NonNull;
use std::sync::mpsc;

use objc2::rc::Retained;
use objc2_foundation::{
    NSData, NSError, NSFileManager, NSHTTPURLResponse, NSMutableURLRequest, NSString,
    NSURLResponse, NSURLSession, NSURLSessionConfiguration, NSURL,
};

use super::release::UpdateError;

/// Seconds a request may wait for the server between bytes. This is
/// `timeoutIntervalForRequest`, not a cap on a transfer in progress, so a
/// slow download of the tarball is not cut off half way.
pub const TIMEOUT_S: f64 = 10.0;

/// Seconds a whole transfer may take, start to finish. This is
/// `timeoutIntervalForResource`: unlike `TIMEOUT_S`, it does not reset when
/// a byte arrives, so it is what bounds a connection that trickles one byte
/// every few seconds and would otherwise never trip the idle timeout. Ten
/// minutes is far more than a ~10 MB tarball needs on a slow link.
const RESOURCE_TIMEOUT_S: f64 = 600.0;

/// A request to `url` carrying exactly `headers`. Nothing else — no
/// token, no cookie. Shared by `fetch` and `download`, which each send a
/// different header set: see their own docs for why.
fn request(
    url: &str,
    headers: &[(&str, &str)],
) -> Result<Retained<NSMutableURLRequest>, UpdateError> {
    let ns_url = NSURL::URLWithString(&NSString::from_str(url))
        .ok_or_else(|| UpdateError::Http(format!("not a URL: {url}")))?;
    let req = NSMutableURLRequest::requestWithURL(&ns_url);
    for (field, value) in headers {
        req.setValue_forHTTPHeaderField(
            Some(&NSString::from_str(value)),
            &NSString::from_str(field),
        );
    }
    Ok(req)
}

/// One session per call: ephemeral (no cache, no cookie jar, nothing on
/// disk) with the request and resource timeouts above. The caller
/// invalidates it.
fn session() -> Retained<NSURLSession> {
    let config = NSURLSessionConfiguration::ephemeralSessionConfiguration();
    config.setTimeoutIntervalForRequest(TIMEOUT_S);
    config.setTimeoutIntervalForResource(RESOURCE_TIMEOUT_S);
    NSURLSession::sessionWithConfiguration(&config)
}

/// Turn the `(response, error)` pair every completion handler receives
/// into `Ok(())` for a 2xx or the reason it was not one.
///
/// # Safety
/// `response` and `error` must be the pointers a completion handler was
/// given, valid for the duration of the call.
unsafe fn status_of(response: *mut NSURLResponse, error: *mut NSError) -> Result<(), String> {
    if let Some(error) = NonNull::new(error) {
        // SAFETY: per this function's contract.
        return Err(unsafe { error.as_ref() }.localizedDescription().to_string());
    }
    let Some(response) = NonNull::new(response) else {
        return Err("no response".to_string());
    };
    // SAFETY: per this function's contract.
    let response = unsafe { response.as_ref() };
    match response.downcast_ref::<NSHTTPURLResponse>() {
        Some(http) => {
            let code = http.statusCode();
            if (200..300).contains(&code) {
                Ok(())
            } else {
                Err(format!("HTTP {code}"))
            }
        }
        None => Err("not an HTTP response".to_string()),
    }
}

/// `GET url`, the whole body in memory. For the `releases/latest` JSON —
/// the one request that actually goes to `api.github.com`, so it is the
/// only one that sends the GitHub REST headers.
pub fn fetch(url: &str) -> Result<Vec<u8>, UpdateError> {
    let user_agent = format!("vitals/{}", env!("CARGO_PKG_VERSION"));
    let req = request(
        url,
        &[
            ("Accept", "application/vnd.github+json"),
            ("X-GitHub-Api-Version", "2022-11-28"),
            ("User-Agent", user_agent.as_str()),
        ],
    )?;
    let session = session();
    let (tx, rx) = mpsc::channel::<Result<Vec<u8>, String>>();
    let handler = block2::RcBlock::new(
        move |data: *mut NSData, response: *mut NSURLResponse, error: *mut NSError| {
            // SAFETY: the three pointers are what NSURLSession hands its
            // completion handler, valid for this call.
            let result = unsafe { status_of(response, error) }.and_then(|()| {
                NonNull::new(data)
                    // SAFETY: as above.
                    .map(|d| unsafe { d.as_ref() }.to_vec())
                    .ok_or_else(|| "empty body".to_string())
            });
            let _ = tx.send(result);
        },
    );
    // SAFETY: `req` is a complete request, and the session retains the
    // block until it has run, so it outlives the task.
    let task = unsafe { session.dataTaskWithRequest_completionHandler(&req, &handler) };
    task.resume();
    let result = rx
        .recv()
        .map_err(|_| UpdateError::Http(format!("{url}: request abandoned")))?;
    session.finishTasksAndInvalidate();
    result.map_err(|reason| UpdateError::Http(format!("{url}: {reason}")))
}

/// `GET url` to a file in `dir`, named after the URL's last path segment.
/// For `SHA256SUMS` and the tarball — both served from
/// `releases/download/…`, which 302s to `objects.githubusercontent.com`
/// (I3): the GitHub REST headers have no business going there, and
/// `Accept: application/vnd.github+json` on a route that content-negotiates
/// on `Accept` is exactly the kind of thing that could turn a redirect
/// into something else. Send only `User-Agent` and a plain binary
/// `Accept`.
pub fn download(url: &str, dir: &Path) -> Result<PathBuf, UpdateError> {
    let name = url
        .rsplit('/')
        .next()
        .filter(|n| !n.is_empty() && !n.contains('?'))
        .ok_or_else(|| UpdateError::Http(format!("no file name at the end of {url}")))?;
    let dest = dir.join(name);
    let user_agent = format!("vitals/{}", env!("CARGO_PKG_VERSION"));
    let req = request(
        url,
        &[
            ("User-Agent", user_agent.as_str()),
            ("Accept", "application/octet-stream"),
        ],
    )?;
    let session = session();
    let (tx, rx) = mpsc::channel::<Result<(), String>>();
    // The block runs on another thread, so it gets a `PathBuf` (Send) and
    // builds its own `NSURL` there, rather than capturing one from here.
    let dest_for_block = dest.clone();
    let handler = block2::RcBlock::new(
        move |location: *mut NSURL, response: *mut NSURLResponse, error: *mut NSError| {
            // SAFETY: the three pointers are what NSURLSession hands its
            // completion handler, valid for this call.
            let result = unsafe { status_of(response, error) }.and_then(|()| {
                let location = NonNull::new(location).ok_or_else(|| "no file".to_string())?;
                // The temporary file is deleted when this block returns,
                // so it is moved now, on this thread.
                let dest =
                    NSURL::fileURLWithPath(&NSString::from_str(&dest_for_block.to_string_lossy()));
                NSFileManager::defaultManager()
                    // SAFETY: as above.
                    .moveItemAtURL_toURL_error(unsafe { location.as_ref() }, &dest)
                    .map_err(|e| e.localizedDescription().to_string())
            });
            let _ = tx.send(result);
        },
    );
    // SAFETY: as in `fetch`.
    let task = unsafe { session.downloadTaskWithRequest_completionHandler(&req, &handler) };
    task.resume();
    let result = rx
        .recv()
        .map_err(|_| UpdateError::Http(format!("{url}: download abandoned")))?;
    session.finishTasksAndInvalidate();
    result
        .map(|()| dest)
        .map_err(|reason| UpdateError::Http(format!("{url}: {reason}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use tiny_http::{Response, Server};

    /// A loopback server answering fixed routes on a background thread
    /// until dropped, recording every request header it sees.
    struct Fixture {
        base: String,
        server: Arc<Server>,
        thread: Option<std::thread::JoinHandle<()>>,
        headers: Arc<Mutex<Vec<(String, String)>>>,
    }

    fn serve(routes: Vec<(&'static str, u16, &'static [u8])>) -> Fixture {
        let server = Arc::new(Server::http("127.0.0.1:0").expect("bind loopback"));
        let port = server.server_addr().to_ip().expect("ip address").port();
        let headers = Arc::new(Mutex::new(Vec::new()));
        let (s, seen) = (Arc::clone(&server), Arc::clone(&headers));
        let thread = std::thread::spawn(move || {
            for request in s.incoming_requests() {
                for h in request.headers() {
                    seen.lock()
                        .unwrap()
                        .push((h.field.to_string(), h.value.to_string()));
                }
                let (status, body) = routes
                    .iter()
                    .find(|(path, _, _)| *path == request.url())
                    .map(|(_, status, body)| (*status, body.to_vec()))
                    .unwrap_or((404, b"not found".to_vec()));
                let _ = request.respond(Response::from_data(body).with_status_code(status));
            }
        });
        Fixture {
            base: format!("http://127.0.0.1:{port}"),
            server,
            thread: Some(thread),
            headers,
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

    /// A port nothing listens on: bind, read the number, close.
    fn closed_port() -> u16 {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.local_addr().unwrap().port()
    }

    #[test]
    fn fetch_returns_the_body_and_sends_exactly_the_github_headers() {
        let fx = serve(vec![(
            "/releases/latest",
            200,
            b"{\"tag_name\":\"v9.9.9\"}",
        )]);
        let body = fetch(&format!("{}/releases/latest", fx.base)).unwrap();
        assert_eq!(body, b"{\"tag_name\":\"v9.9.9\"}");
        let headers = fx.headers.lock().unwrap().clone();
        let find = |name: &str| {
            headers
                .iter()
                .find(|(f, _)| f.eq_ignore_ascii_case(name))
                .map(|(_, v)| v.clone())
        };
        assert_eq!(
            find("Accept").as_deref(),
            Some("application/vnd.github+json")
        );
        assert_eq!(find("X-GitHub-Api-Version").as_deref(), Some("2022-11-28"));
        assert_eq!(
            find("User-Agent").as_deref(),
            Some(format!("vitals/{}", env!("CARGO_PKG_VERSION")).as_str())
        );
        assert!(find("Authorization").is_none());
        assert!(find("Cookie").is_none());
    }

    #[test]
    fn fetch_reports_404_and_403_by_status() {
        let fx = serve(vec![("/forbidden", 403, b"rate limited")]);
        match fetch(&format!("{}/missing", fx.base)) {
            Err(UpdateError::Http(reason)) => assert!(reason.contains("HTTP 404"), "{reason}"),
            other => panic!("expected Http(404), got {other:?}"),
        }
        match fetch(&format!("{}/forbidden", fx.base)) {
            Err(UpdateError::Http(reason)) => assert!(reason.contains("HTTP 403"), "{reason}"),
            other => panic!("expected Http(403), got {other:?}"),
        }
    }

    #[test]
    fn fetch_reports_a_refused_connection() {
        let url = format!("http://127.0.0.1:{}/releases/latest", closed_port());
        match fetch(&url) {
            Err(UpdateError::Http(reason)) => assert!(reason.contains(&url), "{reason}"),
            other => panic!("expected Http(..), got {other:?}"),
        }
    }

    #[test]
    fn fetch_rejects_a_string_that_is_not_a_url() {
        assert!(matches!(
            fetch("not a url at all"),
            Err(UpdateError::Http(_))
        ));
    }

    #[test]
    fn download_writes_the_file_named_after_the_url() {
        let fx = serve(vec![("/Vitals-0.2.0-arm64.tar.gz", 200, b"tarball bytes")]);
        let dir = tempfile::tempdir().unwrap();
        let path = download(
            &format!("{}/Vitals-0.2.0-arm64.tar.gz", fx.base),
            dir.path(),
        )
        .unwrap();
        assert_eq!(path, dir.path().join("Vitals-0.2.0-arm64.tar.gz"));
        assert_eq!(std::fs::read(&path).unwrap(), b"tarball bytes");
    }

    /// (I3) `download` is the one link that really goes to GitHub's
    /// redirect-to-storage route in production; it must not carry the
    /// GitHub REST API headers `fetch` sends.
    #[test]
    fn download_sends_only_user_agent_and_a_plain_accept_header() {
        let fx = serve(vec![("/Vitals-0.2.0-arm64.tar.gz", 200, b"tarball bytes")]);
        let dir = tempfile::tempdir().unwrap();
        download(
            &format!("{}/Vitals-0.2.0-arm64.tar.gz", fx.base),
            dir.path(),
        )
        .unwrap();
        let headers = fx.headers.lock().unwrap().clone();
        let find = |name: &str| {
            headers
                .iter()
                .find(|(f, _)| f.eq_ignore_ascii_case(name))
                .map(|(_, v)| v.clone())
        };
        assert_eq!(find("Accept").as_deref(), Some("application/octet-stream"));
        assert_eq!(
            find("User-Agent").as_deref(),
            Some(format!("vitals/{}", env!("CARGO_PKG_VERSION")).as_str())
        );
        assert!(find("X-GitHub-Api-Version").is_none());
    }

    #[test]
    fn download_of_a_404_leaves_no_file() {
        let fx = serve(vec![]);
        let dir = tempfile::tempdir().unwrap();
        let result = download(&format!("{}/SHA256SUMS", fx.base), dir.path());
        assert!(matches!(result, Err(UpdateError::Http(_))), "{result:?}");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }
}
