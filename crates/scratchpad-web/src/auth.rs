use std::sync::Arc;

use axum::extract::{Request, State};
use axum::http::{header, HeaderMap, Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use base64::Engine;
use scratchpad_core::Scratchpad;

/// Protects the app and API:
/// * rejects cross-site state-changing requests (CSRF) by checking `Origin`,
/// * rejects DNS-rebinding requests to an unauthenticated, loopback-only server,
/// * enforces optional HTTP basic auth when a web username or password is configured.
pub async fn basic_auth(
    State(scratchpad): State<Arc<Scratchpad>>,
    request: Request,
    next: Next,
) -> Response {
    let settings = scratchpad.settings();
    let host = request_host(&request);
    if !matches!(
        *request.method(),
        Method::GET | Method::HEAD | Method::OPTIONS
    ) && is_cross_origin(request.headers(), &host, &settings.public_url)
    {
        return (StatusCode::FORBIDDEN, "Cross-origin request rejected").into_response();
    }
    if !settings.web_auth_enabled() {
        if is_loopback_host(&settings.web_host) && !host_allowed(&host, &settings.public_url) {
            return (StatusCode::FORBIDDEN, "Unexpected Host header").into_response();
        }
        return next.run(request).await;
    }
    let authorized = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| {
            // The scheme name is case-insensitive (RFC 7617).
            let (scheme, credentials) = value.split_once(' ')?;
            scheme.eq_ignore_ascii_case("basic").then_some(credentials)
        })
        .and_then(|encoded| {
            base64::engine::general_purpose::STANDARD
                .decode(encoded.trim())
                .ok()
        })
        .and_then(|decoded| String::from_utf8(decoded).ok())
        .map(|credentials| {
            let (user, pass) = credentials.split_once(':').unwrap_or((&credentials, ""));
            constant_time_eq(user, &settings.web_username)
                & constant_time_eq(pass, &settings.web_password)
        })
        .unwrap_or(false);
    if authorized {
        next.run(request).await
    } else {
        (
            StatusCode::UNAUTHORIZED,
            [(
                header::WWW_AUTHENTICATE,
                "Basic realm=\"Scratchpad\", charset=\"UTF-8\"",
            )],
            "Authentication required",
        )
            .into_response()
    }
}

/// The host (and port) the request was sent to: `Host`, or the URI authority (HTTP/2).
fn request_host(request: &Request) -> String {
    request
        .headers()
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string)
        .or_else(|| request.uri().authority().map(|a| a.to_string()))
        .unwrap_or_default()
}

/// `host[:port]` of a URL such as the configured public URL.
fn url_host(url: &str) -> &str {
    let rest = url.split_once("://").map(|(_, rest)| rest).unwrap_or(url);
    rest.split(['/', '?', '#']).next().unwrap_or_default()
}

/// Browsers always send `Origin` on cross-site POST/PUT/DELETE; scripts (curl, tests) usually
/// don't. The origin must be the host the request was sent to, or the public URL (behind a
/// reverse proxy that rewrites `Host`).
fn is_cross_origin(headers: &HeaderMap, host: &str, public_url: &str) -> bool {
    if headers
        .get("sec-fetch-site")
        .is_some_and(|v| v.as_bytes().eq_ignore_ascii_case(b"cross-site"))
    {
        return true;
    }
    let Some(origin) = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok()) else {
        return false;
    };
    let origin_host = url_host(origin);
    let public_host = url_host(public_url.trim());
    !(origin_host.eq_ignore_ascii_case(host)
        || (!public_host.is_empty() && origin_host.eq_ignore_ascii_case(public_host)))
}

fn is_loopback_host(host: &str) -> bool {
    let host = host.trim_start_matches('[').trim_end_matches(']');
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<std::net::IpAddr>()
            .map(|ip| ip.is_loopback())
            .unwrap_or(false)
}

/// With a loopback-only server, only loopback host names (or the configured public URL) may be used.
fn host_allowed(host: &str, public_url: &str) -> bool {
    if host.is_empty() {
        return true;
    }
    let name = match host.rsplit_once(':') {
        Some((name, port)) if port.chars().all(|c| c.is_ascii_digit()) => name,
        _ => host,
    };
    if is_loopback_host(name) || name.to_ascii_lowercase().ends_with(".localhost") {
        return true;
    }
    let public_host = url_host(public_url.trim());
    !public_host.is_empty()
        && (public_host.eq_ignore_ascii_case(host) || public_host.eq_ignore_ascii_case(name))
}

fn constant_time_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    let mut diff = a.len() ^ b.len();
    for i in 0..a.len().max(b.len()) {
        diff |= usize::from(a.get(i).copied().unwrap_or(0) ^ b.get(i).copied().unwrap_or(0));
    }
    diff == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(pairs: &[(&'static str, &str)]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for (name, value) in pairs {
            map.insert(*name, value.parse().unwrap());
        }
        map
    }

    #[test]
    fn origin_checks() {
        let none = headers(&[]);
        assert!(!is_cross_origin(&none, "127.0.0.1:8025", ""));
        let same = headers(&[("origin", "http://127.0.0.1:8025")]);
        assert!(!is_cross_origin(&same, "127.0.0.1:8025", ""));
        let evil = headers(&[("origin", "https://evil.example")]);
        assert!(is_cross_origin(&evil, "127.0.0.1:8025", ""));
        // Sandboxed frames and file:// pages send `Origin: null`.
        let null = headers(&[("origin", "null")]);
        assert!(is_cross_origin(&null, "127.0.0.1:8025", ""));
        // A reverse proxy that rewrites Host: the public URL's origin is accepted.
        let proxied = headers(&[("origin", "https://scratchpad.example.com")]);
        assert!(!is_cross_origin(
            &proxied,
            "127.0.0.1:8025",
            "https://scratchpad.example.com/"
        ));
        assert!(is_cross_origin(&proxied, "127.0.0.1:8025", ""));
        let fetch_meta = headers(&[("sec-fetch-site", "cross-site")]);
        assert!(is_cross_origin(&fetch_meta, "127.0.0.1:8025", ""));
    }

    #[test]
    fn host_checks() {
        assert!(host_allowed("127.0.0.1:8025", ""));
        assert!(host_allowed("localhost", ""));
        assert!(host_allowed("[::1]:8025", ""));
        assert!(host_allowed("app.localhost:8025", ""));
        assert!(!host_allowed("evil.example", ""));
        assert!(!host_allowed("127.0.0.1.evil.example", ""));
        assert!(host_allowed(
            "scratchpad.example.com",
            "https://scratchpad.example.com"
        ));
        assert!(host_allowed(
            "scratchpad.example.com:8443",
            "https://scratchpad.example.com:8443/x"
        ));
        assert!(!host_allowed(
            "other.example.com",
            "https://scratchpad.example.com"
        ));
    }

    #[test]
    fn constant_time_compare() {
        assert!(constant_time_eq("s3cret", "s3cret"));
        assert!(!constant_time_eq("s3cret", "s3cre"));
        assert!(!constant_time_eq("", "x"));
        assert!(constant_time_eq("", ""));
    }
}
