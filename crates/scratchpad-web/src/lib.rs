//! Web interface for Scratchpad: a REST API, live updates over SSE, public share
//! pages and an embedded single-page app. Used for headless/server deployments
//! and by the desktop app for HTML previews and sharing.

mod api;
mod assets;
mod auth;
mod cloud;
pub mod export;
mod share;

use std::net::SocketAddr;
use std::sync::Arc;

use axum::body::Body;
use axum::extract::Request;
use axum::http::{header, HeaderValue};
use axum::middleware::Next;
use axum::response::Response;
use axum::Router;
use scratchpad_core::Scratchpad;
use tokio::net::TcpListener;

pub fn router(scratchpad: Arc<Scratchpad>) -> Router {
    let protected = Router::new()
        .merge(assets::index_routes())
        .nest("/api", api::routes().merge(cloud::routes()))
        .layer(axum::middleware::from_fn_with_state(
            scratchpad.clone(),
            auth::basic_auth,
        ));
    Router::new()
        .route("/healthz", axum::routing::get(|| async { "ok" }))
        .merge(assets::static_routes())
        .merge(share::routes())
        .merge(protected)
        .layer(axum::middleware::from_fn(response_headers))
        .with_state(scratchpad)
}

/// Headers and error bodies shared by every response:
/// * `nosniff` and `no-referrer` (share tokens are in URLs, and emails load remote images),
/// * `no-store` for API and share responses (they contain email contents),
/// * API errors (including axum's extractor rejections) as `{"error": "..."}` JSON,
/// * no internal error details on the public share pages.
async fn response_headers(request: Request, next: Next) -> Response {
    let path = request.uri().path();
    let public = path.starts_with("/s/");
    let api = path.starts_with("/api/") || (public && path.contains("/api/"));
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers
        .entry(header::REFERRER_POLICY)
        .or_insert(HeaderValue::from_static("no-referrer"));
    if api || public {
        headers
            .entry(header::CACHE_CONTROL)
            .or_insert(HeaderValue::from_static("no-store"));
    }
    let status = response.status();
    let is_json = response
        .headers()
        .get(header::CONTENT_TYPE)
        .is_some_and(|v| v.as_bytes().starts_with(b"application/json"));
    let hide_details = public && status.is_server_error();
    let wrap = api && (status.is_client_error() || status.is_server_error()) && !is_json;
    if !hide_details && !wrap {
        return response;
    }
    let (mut parts, body) = response.into_parts();
    let message = if hide_details {
        "internal error".to_string()
    } else {
        let bytes = axum::body::to_bytes(body, 64 * 1024)
            .await
            .unwrap_or_default();
        let text = String::from_utf8_lossy(&bytes).trim().to_string();
        if text.is_empty() {
            status
                .canonical_reason()
                .unwrap_or("error")
                .to_ascii_lowercase()
        } else {
            text
        }
    };
    let (content_type, body) = if api {
        (
            "application/json",
            serde_json::json!({ "error": message }).to_string(),
        )
    } else {
        ("text/plain; charset=utf-8", message)
    };
    parts.headers.remove(header::CONTENT_LENGTH);
    parts
        .headers
        .insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    Response::from_parts(parts, Body::from(body))
}

/// A bound, not yet serving, web server.
pub struct WebServer {
    listener: TcpListener,
    scratchpad: Arc<Scratchpad>,
}

impl WebServer {
    pub async fn bind(scratchpad: Arc<Scratchpad>, host: &str, port: u16) -> anyhow::Result<Self> {
        let listener = TcpListener::bind((host, port))
            .await
            .map_err(|e| anyhow::anyhow!("could not listen on {host}:{port}: {e}"))?;
        Ok(Self {
            listener,
            scratchpad,
        })
    }

    pub fn local_addr(&self) -> SocketAddr {
        self.listener.local_addr().expect("bound listener")
    }

    /// The URL a browser on this machine can use to reach the server.
    pub fn url(&self) -> String {
        let addr = self.local_addr();
        let host = match addr.ip() {
            ip if ip.is_unspecified() => "127.0.0.1".to_string(),
            std::net::IpAddr::V6(ip) => format!("[{ip}]"),
            ip => ip.to_string(),
        };
        format!("http://{host}:{}", addr.port())
    }

    pub async fn serve(self) -> anyhow::Result<()> {
        self.scratchpad.set_web_url(Some(self.url()));
        tracing::info!("web interface listening on {}", self.url());
        axum::serve(self.listener, router(self.scratchpad)).await?;
        Ok(())
    }
}
