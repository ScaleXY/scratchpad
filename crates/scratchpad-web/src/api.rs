use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderName, HeaderValue, StatusCode};
use axum::response::sse::{Event as SseEvent, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post, put};
use axum::{Json, Router};
use futures::Stream;
use scratchpad_core::{RemoteMailbox, RemoteSource, Scratchpad, Settings};
use serde::Deserialize;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt;

pub type ApiResult<T> = Result<T, ApiError>;

pub struct ApiError(StatusCode, String);

impl ApiError {
    pub fn not_found() -> Self {
        Self(StatusCode::NOT_FOUND, "not found".into())
    }

    pub fn bad_request(message: impl Into<String>) -> Self {
        Self(StatusCode::BAD_REQUEST, message.into())
    }
}

impl<E: Into<anyhow::Error>> From<E> for ApiError {
    fn from(error: E) -> Self {
        let message = format!("{:#}", error.into());
        tracing::error!(error = %message, "request failed");
        Self(StatusCode::INTERNAL_SERVER_ERROR, message)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(serde_json::json!({ "error": self.1 }))).into_response()
    }
}

type AppState = State<Arc<Scratchpad>>;

pub fn routes() -> Router<Arc<Scratchpad>> {
    Router::new()
        .route("/status", get(status))
        .route("/settings", get(get_settings).put(put_settings))
        .route("/mailboxes", get(mailboxes))
        .route("/mailboxes/{name}", delete(delete_mailbox))
        .route("/mailboxes/{name}/clear", post(clear_mailbox))
        .route("/mailboxes/{name}/emails", get(mailbox_emails))
        .route("/emails", get(emails))
        .route("/emails/delete", post(delete_emails))
        .route("/emails/read", post(mark_read))
        .route("/emails/{id}", get(email).delete(delete_email))
        .route("/emails/{id}/raw", get(raw))
        .route("/emails/{id}/html", get(html))
        .route("/emails/{id}/view-data", get(view_data))
        .route("/emails/{id}/attachments/{index}", get(attachment))
        .route("/emails/{id}/forward", post(forward))
        .route("/emails/{id}/recheck", post(recheck))
        .route("/shares", get(shares).post(create_share))
        .route("/shares/{token}", delete(delete_share))
        .route("/remotes", get(remotes).post(save_remote))
        .route("/remotes/poll", post(poll_remotes))
        .route("/remotes/{id}", put(update_remote).delete(delete_remote))
        .route("/test-email", post(test_email))
        .route("/events", get(events))
}

async fn status(State(scratchpad): AppState) -> impl IntoResponse {
    Json(scratchpad.status())
}

async fn get_settings(State(scratchpad): AppState) -> ApiResult<impl IntoResponse> {
    Ok(Json(redacted_settings(&scratchpad.settings())?))
}

/// Passwords are write-only, like S3 secrets: they come back empty with a `*_set` flag,
/// and a missing or `null` password in an update keeps the current one.
fn redacted_settings(settings: &Settings) -> anyhow::Result<serde_json::Value> {
    let mut value = serde_json::to_value(settings)?;
    value["web_password_set"] = (!settings.web_password.is_empty()).into();
    value["web_password"] = "".into();
    value["forward"]["password_set"] = (!settings.forward.password.is_empty()).into();
    value["forward"]["password"] = "".into();
    Ok(value)
}

async fn put_settings(
    State(scratchpad): AppState,
    Json(mut body): Json<serde_json::Value>,
) -> ApiResult<impl IntoResponse> {
    let current = scratchpad.settings();
    if body.get("web_password").is_none_or(|v| v.is_null()) {
        body["web_password"] = current.web_password.clone().into();
    }
    if let Some(forward) = body.get_mut("forward").filter(|f| f.is_object()) {
        if forward.get("password").is_none_or(|v| v.is_null()) {
            forward["password"] = current.forward.password.clone().into();
        }
    }
    let settings: Settings = serde_json::from_value(body)
        .map_err(|e| ApiError::bad_request(format!("invalid settings: {e}")))?;
    validate_settings(&settings).map_err(ApiError::bad_request)?;
    scratchpad.update_settings(settings)?;
    Ok(Json(redacted_settings(&scratchpad.settings())?))
}

fn validate_settings(s: &Settings) -> Result<(), String> {
    let ports = [
        ("SMTP port", s.smtp_port),
        ("Web port", s.web_port),
        ("spamd port", s.spamd_port),
    ];
    if let Some((name, _)) = ports.iter().find(|(_, port)| *port == 0) {
        return Err(format!("{name} must be between 1 and 65535"));
    }
    if s.smtp_host.trim().is_empty() || s.web_host.trim().is_empty() {
        return Err("SMTP and web hosts can't be empty".into());
    }
    if !s.forward.host.trim().is_empty() && s.forward.port == 0 {
        return Err("Forwarding port must be between 1 and 65535".into());
    }
    let public_url = s.public_url.trim();
    let valid_url = public_url.is_empty()
        || public_url.starts_with("http://")
        || public_url.starts_with("https://");
    if !valid_url {
        return Err("Public URL must start with http:// or https://".into());
    }
    if s.max_message_size < 1024 {
        return Err("Max message size must be at least 1024 bytes".into());
    }
    if !(1..=300).contains(&s.link_checker_timeout_secs) {
        return Err("Link timeout must be between 1 and 300 seconds".into());
    }
    if s.remote_poll_secs == 0 {
        return Err("Remote poll interval must be at least 1 second".into());
    }
    Ok(())
}

async fn mailboxes(State(scratchpad): AppState) -> ApiResult<impl IntoResponse> {
    Ok(Json(scratchpad.mailboxes()?))
}

async fn delete_mailbox(
    State(scratchpad): AppState,
    Path(name): Path<String>,
) -> ApiResult<StatusCode> {
    scratchpad.delete_mailbox(&name)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn clear_mailbox(
    State(scratchpad): AppState,
    Path(name): Path<String>,
) -> ApiResult<StatusCode> {
    scratchpad.clear_mailbox(&name)?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct ListQuery {
    q: Option<String>,
    mailbox: Option<String>,
    // Not `#[serde(flatten)] Page`: flattened numbers don't parse from query strings.
    limit: Option<usize>,
    #[serde(default)]
    offset: usize,
}

impl ListQuery {
    fn page(&self) -> Page {
        Page {
            limit: self.limit,
            offset: self.offset,
        }
    }
}

/// Optional `limit`/`offset` for email lists. The total is sent in `X-Total-Count`.
#[derive(Deserialize, Default)]
pub struct Page {
    pub limit: Option<usize>,
    #[serde(default)]
    pub offset: usize,
}

impl Page {
    pub fn apply<T>(&self, items: Vec<T>) -> Vec<T> {
        let rest = items.into_iter().skip(self.offset);
        match self.limit {
            Some(limit) => rest.take(limit).collect(),
            None => rest.collect(),
        }
    }
}

/// Lists emails; paging happens in SQLite so large mailboxes stay cheap.
fn email_list(
    scratchpad: &Scratchpad,
    mailbox: Option<&str>,
    query: &ListQuery,
) -> ApiResult<Response> {
    let page = query.page();
    let (emails, total) =
        scratchpad.emails_page(mailbox, query.q.as_deref(), page.limit, page.offset)?;
    Ok((
        [(HeaderName::from_static("x-total-count"), total.to_string())],
        Json(emails),
    )
        .into_response())
}

async fn mailbox_emails(
    State(scratchpad): AppState,
    Path(name): Path<String>,
    Query(query): Query<ListQuery>,
) -> ApiResult<Response> {
    email_list(&scratchpad, Some(&name), &query)
}

async fn emails(
    State(scratchpad): AppState,
    Query(query): Query<ListQuery>,
) -> ApiResult<Response> {
    email_list(&scratchpad, query.mailbox.as_deref(), &query)
}

#[derive(Deserialize)]
struct Ids {
    ids: Vec<String>,
    #[serde(default = "yes")]
    read: bool,
}

fn yes() -> bool {
    true
}

async fn delete_emails(
    State(scratchpad): AppState,
    Json(body): Json<Ids>,
) -> ApiResult<StatusCode> {
    scratchpad.delete_emails(&body.ids)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn mark_read(State(scratchpad): AppState, Json(body): Json<Ids>) -> ApiResult<StatusCode> {
    scratchpad.mark_read(&body.ids, body.read)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn email(
    State(scratchpad): AppState,
    Path(id): Path<String>,
) -> ApiResult<impl IntoResponse> {
    Ok(Json(
        scratchpad.email(&id)?.ok_or_else(ApiError::not_found)?,
    ))
}

async fn delete_email(
    State(scratchpad): AppState,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    scratchpad.delete_emails(&[id])?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct RawQuery {
    #[serde(default)]
    pub download: bool,
}

async fn raw(
    State(scratchpad): AppState,
    Path(id): Path<String>,
    Query(query): Query<RawQuery>,
) -> ApiResult<Response> {
    raw_response(&scratchpad, &id, query.download)
}

pub fn raw_response(scratchpad: &Scratchpad, id: &str, download: bool) -> ApiResult<Response> {
    let raw = scratchpad.raw(id)?.ok_or_else(ApiError::not_found)?;
    let mut response = raw.into_response();
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static("default-src 'none'; sandbox"),
    );
    if download {
        headers.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("message/rfc822"),
        );
        let filename = scratchpad
            .summary(id)?
            .map(|s| clean_filename(&s.subject))
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| id.to_string());
        headers.insert(
            header::CONTENT_DISPOSITION,
            content_disposition("attachment", &format!("{filename}.eml")),
        );
    } else {
        headers.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("text/plain; charset=utf-8"),
        );
    }
    Ok(response)
}

/// Makes a name from an email (subject, attachment name) safe to save: no path
/// separators, control or reserved characters, at most 120 characters.
pub(crate) fn clean_filename(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') {
                '_'
            } else {
                c
            }
        })
        .take(120)
        .collect();
    cleaned
        .trim_matches(|c: char| c == '.' || c.is_whitespace())
        .to_string()
}

/// A `Content-Disposition` header (RFC 6266): an ASCII `filename` fallback plus the
/// UTF-8 name in `filename*`, so non-ASCII names survive and nothing can be injected.
pub(crate) fn content_disposition(kind: &'static str, filename: &str) -> HeaderValue {
    let filename = clean_filename(filename);
    let fallback: String = filename
        .chars()
        .map(|c| match c {
            ' ' => ' ',
            '%' | ';' | '\'' => '_',
            c if c.is_ascii_graphic() => c,
            _ => '_',
        })
        .collect();
    let mut encoded = String::new();
    for byte in filename.bytes() {
        if byte.is_ascii_alphanumeric() || b"!#$&+-.^_`|~".contains(&byte) {
            encoded.push(byte as char);
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    let value = if filename.is_empty() {
        kind.to_string()
    } else {
        format!("{kind}; filename=\"{fallback}\"; filename*=UTF-8''{encoded}")
    };
    HeaderValue::from_str(&value).unwrap_or_else(|_| HeaderValue::from_static(kind))
}

/// Content Security Policy for rendered email HTML: no scripts or forms, remote images
/// allowed. `sandbox` keeps it inert even when opened in its own tab.
const EMAIL_CSP: &str = "default-src 'none'; img-src * data: blob:; style-src * 'unsafe-inline'; font-src * data:; media-src *; form-action 'none'; frame-ancestors 'self'; sandbox allow-popups allow-popups-to-escape-sandbox allow-same-origin";

async fn html(State(scratchpad): AppState, Path(id): Path<String>) -> ApiResult<Response> {
    html_response(&scratchpad, &id)
}

pub fn html_response(scratchpad: &Scratchpad, id: &str) -> ApiResult<Response> {
    let raw = scratchpad.raw(id)?.ok_or_else(ApiError::not_found)?;
    let html = scratchpad_core::parse::html_with_cids(&raw, |index| format!("attachments/{index}"))
        .unwrap_or_else(|| {
            let detail = scratchpad_core::parse::parse(&raw).and_then(|m| m.body_text(0).map(|t| t.into_owned())).unwrap_or_default();
            format!(
                "<pre style=\"white-space:pre-wrap;font:14px/1.5 ui-monospace,monospace;margin:16px\">{}</pre>",
                escape_html(&detail)
            )
        });
    Ok(sandboxed_html(&html))
}

pub fn sandboxed_html(html: &str) -> Response {
    // Open links in a new tab and resolve cid: attachment URLs relative to the email.
    let injected = "<base target=\"_blank\">";
    let body = match html.to_ascii_lowercase().find("<head") {
        Some(start) => match html[start..].find('>') {
            Some(end) => {
                let at = start + end + 1;
                format!("{}{injected}{}", &html[..at], &html[at..])
            }
            None => format!("{injected}{html}"),
        },
        None => format!("{injected}{html}"),
    };
    (
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8"),
            (header::CONTENT_SECURITY_POLICY, EMAIL_CSP),
            (header::CACHE_CONTROL, "no-store"),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
        ],
        body,
    )
        .into_response()
}

pub fn escape_html(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

async fn view_data(State(scratchpad): AppState, Path(id): Path<String>) -> ApiResult<Response> {
    view_data_response(&scratchpad, &id)
}

pub fn view_data_response(scratchpad: &Scratchpad, id: &str) -> ApiResult<Response> {
    let detail = scratchpad.email(id)?.ok_or_else(ApiError::not_found)?;
    let html = detail
        .view_inspector
        .and_then(|inspector| inspector.view_data_html)
        .ok_or_else(ApiError::not_found)?;
    // Symfony's HtmlDumper output relies on its own inline script for collapsing. The
    // HTML comes from an email header, so it always runs in a sandbox with an opaque
    // origin, even when opened directly (e.g. through a relative link in the email).
    Ok((
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8"),
            (header::CONTENT_SECURITY_POLICY, "default-src 'none'; style-src 'unsafe-inline'; script-src 'unsafe-inline'; form-action 'none'; frame-ancestors 'self'; sandbox allow-scripts"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        format!("<!doctype html><meta charset=utf-8><body style=\"margin:0;background:#18171B\">{html}</body>"),
    )
        .into_response())
}

async fn attachment(
    State(scratchpad): AppState,
    Path((id, index)): Path<(String, usize)>,
) -> ApiResult<Response> {
    attachment_response(&scratchpad, &id, index)
}

pub fn attachment_response(scratchpad: &Scratchpad, id: &str, index: usize) -> ApiResult<Response> {
    let (meta, bytes) = scratchpad
        .attachment(id, index)?
        .ok_or_else(ApiError::not_found)?;
    let mut response = bytes.into_response();
    let headers = response.headers_mut();
    if let Ok(value) = HeaderValue::from_str(&meta.content_type) {
        headers.insert(header::CONTENT_TYPE, value);
    }
    let disposition = if meta.inline { "inline" } else { "attachment" };
    headers.insert(
        header::CONTENT_DISPOSITION,
        content_disposition(disposition, &meta.filename),
    );
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    // Attachments are untrusted (an HTML or SVG file could carry scripts or forms), so
    // they open sandboxed. Chrome's PDF viewer refuses to run in a sandbox; PDFs can't
    // script the page, so they are left out.
    let csp = if meta.content_type.eq_ignore_ascii_case("application/pdf") {
        "default-src 'none'; img-src 'self' data:; style-src 'unsafe-inline'; form-action 'none'; frame-ancestors 'self'"
    } else {
        "default-src 'none'; img-src 'self' data:; style-src 'unsafe-inline'; media-src 'self'; form-action 'none'; frame-ancestors 'self'; sandbox"
    };
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(csp),
    );
    Ok(response)
}

#[derive(Deserialize)]
struct ForwardBody {
    to: Vec<String>,
}

async fn forward(
    State(scratchpad): AppState,
    Path(id): Path<String>,
    Json(body): Json<ForwardBody>,
) -> ApiResult<StatusCode> {
    let recipients: Vec<String> = body
        .to
        .into_iter()
        .map(|r| r.trim().to_string())
        .filter(|r| !r.is_empty())
        .collect();
    if recipients.is_empty() {
        return Err(ApiError::bad_request("enter at least one recipient"));
    }
    scratchpad.summary(&id)?.ok_or_else(ApiError::not_found)?;
    scratchpad
        .forward(&id, &recipients)
        .await
        .map_err(|e| ApiError::bad_request(format!("{e:#}")))?;
    Ok(StatusCode::NO_CONTENT)
}

async fn recheck(State(scratchpad): AppState, Path(id): Path<String>) -> ApiResult<StatusCode> {
    scratchpad.summary(&id)?.ok_or_else(ApiError::not_found)?;
    // Parses the message synchronously before spawning the checks.
    tokio::task::spawn_blocking(move || scratchpad.recheck(&id))
        .await
        .map_err(anyhow::Error::from)?;
    Ok(StatusCode::ACCEPTED)
}

async fn shares(State(scratchpad): AppState) -> ApiResult<impl IntoResponse> {
    let shares: Vec<_> = scratchpad
        .shares()?
        .into_iter()
        .map(|share| {
            let url = scratchpad.share_url(&share.token);
            serde_json::json!({ "share": share, "url": url })
        })
        .collect();
    Ok(Json(shares))
}

#[derive(Deserialize)]
struct CreateShare {
    title: String,
    #[serde(default)]
    ids: Vec<String>,
    /// Share the whole mailbox (live) instead of a fixed list of emails.
    #[serde(default)]
    mailbox: Option<String>,
}

async fn create_share(
    State(scratchpad): AppState,
    Json(body): Json<CreateShare>,
) -> ApiResult<impl IntoResponse> {
    for id in &body.ids {
        scratchpad.summary(id)?.ok_or_else(ApiError::not_found)?;
    }
    let share = match &body.mailbox {
        Some(mailbox) => scratchpad.create_mailbox_share(&body.title, mailbox),
        None => scratchpad.create_share(&body.title, body.ids),
    }
    .map_err(|e| ApiError::bad_request(e.to_string()))?;
    let url = scratchpad.share_url(&share.token);
    Ok(Json(serde_json::json!({ "share": share, "url": url })))
}

async fn delete_share(
    State(scratchpad): AppState,
    Path(token): Path<String>,
) -> ApiResult<StatusCode> {
    scratchpad.delete_share(&token)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn remotes(State(scratchpad): AppState) -> ApiResult<impl IntoResponse> {
    let remotes = scratchpad
        .remotes()?
        .iter()
        .map(redacted_remote)
        .collect::<anyhow::Result<Vec<_>>>()?;
    Ok(Json(remotes))
}

/// Remote mailbox passwords are write-only: sent back as `null` with `has_password`.
fn redacted_remote(remote: &RemoteMailbox) -> anyhow::Result<serde_json::Value> {
    let has_password = match &remote.source {
        RemoteSource::Imap { password, .. } => !password.is_empty(),
        RemoteSource::Scratchpad { password, .. } => {
            password.as_ref().is_some_and(|p| !p.is_empty())
        }
    };
    let mut value = serde_json::to_value(remote)?;
    value["source"]["password"] = serde_json::Value::Null;
    value["source"]["has_password"] = has_password.into();
    Ok(value)
}

/// Parses a remote from the UI. A missing or `null` password keeps the stored one.
fn remote_from_body(
    scratchpad: &Scratchpad,
    id: &str,
    mut body: serde_json::Value,
) -> ApiResult<RemoteMailbox> {
    if body["source"].get("password").is_none_or(|p| p.is_null()) {
        let stored = scratchpad.remotes()?.into_iter().find(|r| r.id == id);
        let password = match stored.map(|r| r.source) {
            Some(RemoteSource::Imap { password, .. }) if body["source"]["type"] == "imap" => {
                Some(password)
            }
            Some(RemoteSource::Scratchpad { password, .. })
                if body["source"]["type"] == "scratchpad" =>
            {
                password
            }
            _ => None,
        };
        if let Some(source) = body.get_mut("source").filter(|s| s.is_object()) {
            source["password"] = match password {
                Some(password) => password.into(),
                None if source["type"] == "imap" => "".into(),
                None => serde_json::Value::Null,
            };
        }
    }
    let mut remote: RemoteMailbox = serde_json::from_value(body)
        .map_err(|e| ApiError::bad_request(format!("invalid remote mailbox: {e}")))?;
    remote.id = id.to_string();
    Ok(remote)
}

async fn save_remote(
    State(scratchpad): AppState,
    Json(body): Json<serde_json::Value>,
) -> ApiResult<impl IntoResponse> {
    let id = body["id"].as_str().unwrap_or_default().to_string();
    let remote = remote_from_body(&scratchpad, &id, body)?;
    let saved = scratchpad
        .save_remote(remote)
        .map_err(|e| ApiError::bad_request(e.to_string()))?;
    Ok(Json(redacted_remote(&saved)?))
}

async fn update_remote(
    State(scratchpad): AppState,
    Path(id): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> ApiResult<impl IntoResponse> {
    if !scratchpad.remotes()?.iter().any(|r| r.id == id) {
        return Err(ApiError::not_found());
    }
    let remote = remote_from_body(&scratchpad, &id, body)?;
    let saved = scratchpad
        .save_remote(remote)
        .map_err(|e| ApiError::bad_request(e.to_string()))?;
    Ok(Json(redacted_remote(&saved)?))
}

#[derive(Deserialize)]
struct DeleteRemoteQuery {
    #[serde(default)]
    delete_emails: bool,
}

async fn delete_remote(
    State(scratchpad): AppState,
    Path(id): Path<String>,
    Query(query): Query<DeleteRemoteQuery>,
) -> ApiResult<StatusCode> {
    scratchpad.delete_remote(&id, query.delete_emails)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn poll_remotes(State(scratchpad): AppState) -> StatusCode {
    scratchpad.poll_remotes_now();
    StatusCode::ACCEPTED
}

#[derive(Deserialize)]
struct TestEmail {
    #[serde(default)]
    mailbox: Option<String>,
}

async fn test_email(
    State(scratchpad): AppState,
    Json(body): Json<TestEmail>,
) -> ApiResult<StatusCode> {
    let mailbox = body
        .mailbox
        .filter(|m| !m.trim().is_empty())
        .unwrap_or_else(|| "Test".into());
    scratchpad
        .send_test_email(&mailbox)
        .await
        .map_err(|e| ApiError::bad_request(format!("{e:#}")))?;
    Ok(StatusCode::NO_CONTENT)
}

async fn events(
    State(scratchpad): AppState,
) -> Sse<impl Stream<Item = Result<SseEvent, Infallible>>> {
    let stream = BroadcastStream::new(scratchpad.subscribe()).filter_map(|event| match event {
        Ok(event) => SseEvent::default().json_data(&event).ok().map(Ok),
        // A slow client missed events; tell it to reload everything.
        Err(_) => Some(Ok(SseEvent::default().data("{\"type\":\"resync\"}"))),
    });
    Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(15)))
}
