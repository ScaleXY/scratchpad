use std::sync::Arc;

use axum::http::header;
use axum::response::{Html, IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use scratchpad_core::Scratchpad;

const INDEX: &str = include_str!("../assets/index.html");
const APP_JS: &str = include_str!("../assets/app.js");
const BOOT_JS: &str = include_str!("../assets/boot.js");
pub(crate) const APP_CSS: &str = include_str!("../assets/app.css");
pub(crate) const DEVICES_JS: &str = include_str!("../assets/devices.js");
pub(crate) const DEVICES_CSS: &str = include_str!("../assets/devices.css");
pub(crate) const ICON: &str = include_str!("../assets/icon.svg");
pub(crate) const ICONS: &str = include_str!("../assets/icons.svg");
pub(crate) const FONTS: [(&str, &[u8]); 4] = [
    (
        "inter-400.woff2",
        include_bytes!("../assets/fonts/inter-400.woff2"),
    ),
    (
        "inter-500.woff2",
        include_bytes!("../assets/fonts/inter-500.woff2"),
    ),
    (
        "inter-600.woff2",
        include_bytes!("../assets/fonts/inter-600.woff2"),
    ),
    (
        "inter-700.woff2",
        include_bytes!("../assets/fonts/inter-700.woff2"),
    ),
];

/// Content Security Policy for the app shell: only its own scripts, styles (plus inline
/// `style` attributes), images, fonts, frames and API.
const APP_CSP: &str = "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; font-src 'self'; connect-src 'self'; frame-src 'self'; object-src 'none'; base-uri 'none'; form-action 'self'; frame-ancestors 'none'";

/// The app shell. `share_token` is empty for the app itself, or a share page's token.
pub fn app_page(share_token: &str) -> Response {
    // Tokens are alphanumeric; filter anyway since the value lands in an attribute.
    let token: String = share_token
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect();
    (
        [
            (header::CONTENT_SECURITY_POLICY, APP_CSP),
            (header::CACHE_CONTROL, "no-store"),
        ],
        Html(INDEX.replace("__SHARE_TOKEN__", &token)),
    )
        .into_response()
}

/// The app shell (behind auth).
pub fn index_routes() -> Router<Arc<Scratchpad>> {
    Router::new().route("/", get(|| async { app_page("") }))
}

fn script(source: &'static str) -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "text/javascript; charset=utf-8"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        source,
    )
}

fn stylesheet(source: &'static str) -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "text/css; charset=utf-8"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        source,
    )
}

/// Static files, public so share pages can load them.
pub fn static_routes() -> Router<Arc<Scratchpad>> {
    Router::new()
        .route("/assets/boot.js", get(|| async { script(BOOT_JS) }))
        .route("/assets/app.js", get(|| async { script(APP_JS) }))
        .route("/assets/app.css", get(|| async { stylesheet(APP_CSS) }))
        .route("/assets/devices.js", get(|| async { script(DEVICES_JS) }))
        .route(
            "/assets/devices.css",
            get(|| async { stylesheet(DEVICES_CSS) }),
        )
        .route("/assets/icon.svg", get(icon))
        .route("/assets/fonts/{name}", get(font))
        .route(
            "/assets/icons.svg",
            get(|| async {
                (
                    [
                        (header::CONTENT_TYPE, "image/svg+xml"),
                        (header::CACHE_CONTROL, "max-age=3600"),
                    ],
                    ICONS,
                )
            }),
        )
}

pub async fn icon() -> impl IntoResponse {
    ([(header::CONTENT_TYPE, "image/svg+xml")], ICON)
}

async fn font(axum::extract::Path(name): axum::extract::Path<String>) -> axum::response::Response {
    match FONTS.iter().find(|(file, _)| *file == name) {
        Some((_, bytes)) => (
            [
                (header::CONTENT_TYPE, "font/woff2"),
                (header::CACHE_CONTROL, "max-age=31536000, immutable"),
            ],
            *bytes,
        )
            .into_response(),
        None => axum::http::StatusCode::NOT_FOUND.into_response(),
    }
}
