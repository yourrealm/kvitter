//! Who is asking, and what anonymous visitors may reach.

use axum::{
    extract::{Request, State},
    http::{HeaderMap, HeaderValue, Method, StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Redirect, Response},
};

use rand::RngExt;

use crate::{AppState, visibility::Viewer};

pub const USER_HEADER: &str = "x-kvitter-user";
pub const PROOF_HEADER: &str = "x-kvitter-proof";

tokio::task_local! {
    static NONCE: String;
}

/// This response's CSP nonce. CodeMirror needs it for the `<style>` elements it injects.
pub fn nonce() -> String {
    NONCE.try_with(String::clone).unwrap_or_default()
}

/// Sent by the app rather than the Realm router: the nonce changes per response, and this way
/// it also applies in local dev and e2e tests.
fn csp(nonce: &str) -> String {
    format!(
        "default-src 'self'; img-src 'self' data:; style-src 'self' 'nonce-{nonce}'; \
         frame-ancestors 'self'; base-uri 'self'; form-action 'self'"
    )
}

/// The user header counts only when Realm's proof header matches `KVITTER_PROOF`.
/// No proof configured means nobody is ever signed in.
pub fn viewer_from_headers(headers: &HeaderMap, proof: Option<&str>) -> Viewer {
    let Some(expected) = proof else { return Viewer::Anonymous };
    let sent = headers.get(PROOF_HEADER).map(|v| v.as_bytes()).unwrap_or_default();
    if !constant_time_eq(sent, expected.as_bytes()) {
        return Viewer::Anonymous;
    }
    match headers.get(USER_HEADER).and_then(|v| v.to_str().ok()).map(str::trim) {
        Some(u) if !u.is_empty() => Viewer::User(u.to_string()),
        _ => Viewer::Anonymous,
    }
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn dev_viewer(headers: &HeaderMap, user: &str) -> Viewer {
    let host = headers.get(header::HOST).and_then(|h| h.to_str().ok()).unwrap_or_default();
    if host.starts_with("public.") { Viewer::Anonymous } else { Viewer::User(user.to_string()) }
}

/// Paths anonymous visitors may reach. Anonymous requests only arrive via the public domain
/// (the private URL is behind Home's login), so this is what makes the public domain read-only.
fn anonymous_allowed(method: &Method, path: &str) -> bool {
    if method != Method::GET && method != Method::HEAD {
        return false;
    }
    matches!(path, "/explore" | "/rss.xml" | "/sitemap.xml" | "/robots.txt" | "/healthz")
        || ["/p/", "/files/", "/static/"].iter().any(|prefix| {
            path.strip_prefix(prefix).is_some_and(|rest| !rest.is_empty() && !rest.contains('/'))
        })
}

pub async fn identify(State(state): State<AppState>, req: Request, next: Next) -> Response {
    let nonce: String = {
        let mut rng = rand::rng();
        (0..16).map(|_| format!("{:02x}", rng.random::<u8>())).collect()
    };
    let mut res = NONCE.scope(nonce.clone(), gate(state, req, next)).await;
    if let Ok(v) = HeaderValue::from_str(&csp(&nonce)) {
        res.headers_mut().insert(header::CONTENT_SECURITY_POLICY, v);
    }
    res
}

async fn gate(state: AppState, mut req: Request, next: Next) -> Response {
    let viewer = match &state.config.dev_user {
        Some(user) => dev_viewer(req.headers(), user),
        None => viewer_from_headers(req.headers(), state.config.proof.as_deref()),
    };

    // Cross-site form posts would ride on the Home session cookie. Other Realm apps share the
    // site, so `same-site` is refused too.
    if req.method() != Method::GET && req.method() != Method::HEAD {
        let site = req.headers().get("sec-fetch-site").and_then(|v| v.to_str().ok());
        if matches!(site, Some("cross-site" | "same-site")) {
            return (StatusCode::FORBIDDEN, "Cross-site request refused").into_response();
        }
    }

    if !viewer.is_signed_in() {
        if req.uri().path() == "/" && req.method() == Method::GET {
            return Redirect::to("/explore").into_response();
        }
        if !anonymous_allowed(req.method(), req.uri().path()) {
            return crate::routes::not_found(&state, &viewer);
        }
    }

    let is_static = req.uri().path().starts_with("/static/");
    req.extensions_mut().insert(viewer.clone());
    let mut res = next.run(req).await;

    // Pages for a signed-in viewer are never cached. Anonymous responses may be, unless the
    // handler already decided otherwise. Static assets are the same for everyone.
    let headers = res.headers_mut();
    if viewer.is_signed_in() && !is_static {
        headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("private, no-store"));
    } else if !headers.contains_key(header::CACHE_CONTROL) {
        headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("public, max-age=60"));
    }
    res
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(pairs: &[(&'static str, &'static str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.insert(*k, HeaderValue::from_static(v));
        }
        h
    }

    #[test]
    fn user_header_needs_matching_proof() {
        let ok = headers(&[(USER_HEADER, "max"), (PROOF_HEADER, "s3cret")]);
        assert_eq!(viewer_from_headers(&ok, Some("s3cret")), Viewer::User("max".into()));

        let wrong = headers(&[(USER_HEADER, "max"), (PROOF_HEADER, "guess")]);
        assert_eq!(viewer_from_headers(&wrong, Some("s3cret")), Viewer::Anonymous);

        let missing = headers(&[(USER_HEADER, "max")]);
        assert_eq!(viewer_from_headers(&missing, Some("s3cret")), Viewer::Anonymous);

        assert_eq!(viewer_from_headers(&ok, None), Viewer::Anonymous, "no proof configured");
    }

    #[test]
    fn anonymous_routes() {
        let get = Method::GET;
        for ok in ["/explore", "/p/k7mq3xzh", "/files/k7mq3xzh", "/static/app.css", "/rss.xml"] {
            assert!(anonymous_allowed(&get, ok), "{ok}");
        }
        for no in ["/", "/u/max", "/t/sauna", "/p/k7mq3xzh/edit", "/api/tags", "/p/"] {
            assert!(!anonymous_allowed(&get, no), "{no}");
        }
        assert!(!anonymous_allowed(&Method::POST, "/p/k7mq3xzh"));
    }
}
