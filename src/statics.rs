//! CSS, JS and the favicon, embedded in the binary. URLs carry a content hash so they can be
//! cached for a long time. `composer.js` is built by `pnpm build:web` before `cargo build`.

use std::{
    hash::{DefaultHasher, Hash, Hasher},
    sync::OnceLock,
};

use axum::{
    extract::Path,
    http::{HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};

const FILES: &[(&str, &str, &[u8])] = &[
    ("app.css", "text/css; charset=utf-8", include_bytes!("../static/app.css")),
    ("app.js", "text/javascript; charset=utf-8", include_bytes!("../static/app.js")),
    ("composer.js", "text/javascript; charset=utf-8", include_bytes!("../static/composer.js")),
    ("favicon.svg", "image/svg+xml", include_bytes!("../static/favicon.svg")),
];

fn version() -> &'static str {
    static V: OnceLock<String> = OnceLock::new();
    V.get_or_init(|| {
        let mut h = DefaultHasher::new();
        for (_, _, bytes) in FILES {
            bytes.hash(&mut h);
        }
        format!("{:x}", h.finish() & 0xffff_ffff)
    })
}

pub fn url(name: &str) -> String {
    format!("/static/{name}?v={}", version())
}

pub async fn serve(Path(name): Path<String>) -> Response {
    let Some((_, content_type, bytes)) = FILES.iter().find(|(n, _, _)| *n == name) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let mut res = bytes.into_response();
    let headers = res.headers_mut();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("public, max-age=31536000, immutable"));
    res
}
