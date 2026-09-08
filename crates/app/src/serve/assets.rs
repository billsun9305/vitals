//! Placeholder asset serving. Replaced wholesale in Task 19 by the built
//! dashboard's embedded assets.

use std::io::Cursor;
use tiny_http::Response;

pub fn index() -> Response<Cursor<Vec<u8>>> {
    Response::from_string("<!doctype html><title>vitals</title><p>dashboard not built yet")
        .with_header(
            tiny_http::Header::from_bytes(&b"Content-Type"[..], &b"text/html; charset=utf-8"[..])
                .unwrap(),
        )
}

pub fn serve(_path: &str) -> Response<Cursor<Vec<u8>>> {
    Response::from_string("not found").with_status_code(404)
}
