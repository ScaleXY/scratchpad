//! API for HTML exports and cloud sharing (S3 configs, credential vault, cloud shares).

use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use scratchpad_core::{S3Config, Scratchpad};
use serde::Deserialize;

use crate::api::{content_disposition, ApiError, ApiResult};
use crate::export::{self, CloudShareRequest, ExportOptions};

type AppState = State<Arc<Scratchpad>>;

pub fn routes() -> Router<Arc<Scratchpad>> {
    Router::new()
        .route("/export", get(export_get).post(export_post))
        .route("/emails/{id}/export", get(export_one))
        .route("/cloud", get(overview))
        .route("/cloud/configs", post(save_config))
        .route("/cloud/configs/test", post(test_config))
        .route("/cloud/configs/{id}", delete(delete_config))
        .route("/cloud/vault", delete(vault_reset))
        .route("/cloud/vault/create", post(vault_create))
        .route("/cloud/vault/unlock", post(vault_unlock))
        .route("/cloud/vault/lock", post(vault_lock))
        .route("/cloud/vault/password", post(vault_password))
        .route("/cloud/vault/sync", post(vault_sync))
        .route("/cloud/vault/restore", post(vault_restore))
        .route("/cloud/shares", get(shares).post(create_share))
        .route("/cloud/shares/{id}", delete(delete_share))
        .route("/cloud/shares/{id}/renew", post(renew_share))
}

fn user_error(error: anyhow::Error) -> ApiError {
    ApiError::bad_request(format!("{error:#}"))
}

// ---- export -----------------------------------------------------------------

#[derive(Deserialize)]
struct ExportQuery {
    /// Comma-separated email ids.
    #[serde(default)]
    ids: String,
    title: Option<String>,
    attachments: Option<bool>,
    technical: Option<bool>,
    device: Option<String>,
}

impl ExportQuery {
    fn options(&self) -> ExportOptions {
        let defaults = ExportOptions::default();
        ExportOptions {
            attachments: self.attachments.unwrap_or(defaults.attachments),
            technical: self.technical.unwrap_or(defaults.technical),
            device: self.device.clone(),
        }
    }
}

#[derive(Deserialize)]
struct ExportBody {
    ids: Vec<String>,
    title: Option<String>,
    #[serde(default)]
    options: ExportOptions,
}

async fn export_get(
    State(scratchpad): AppState,
    Query(q): Query<ExportQuery>,
) -> ApiResult<Response> {
    let ids: Vec<String> = q
        .ids
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect();
    download(scratchpad, ids, q.title.clone(), q.options()).await
}

async fn export_post(
    State(scratchpad): AppState,
    Json(body): Json<ExportBody>,
) -> ApiResult<Response> {
    download(scratchpad, body.ids, body.title, body.options).await
}

async fn export_one(
    State(scratchpad): AppState,
    Path(id): Path<String>,
    Query(q): Query<ExportQuery>,
) -> ApiResult<Response> {
    download(scratchpad, vec![id], q.title.clone(), q.options()).await
}

async fn download(
    scratchpad: Arc<Scratchpad>,
    ids: Vec<String>,
    title: Option<String>,
    options: ExportOptions,
) -> ApiResult<Response> {
    for id in &ids {
        scratchpad.summary(id)?.ok_or_else(ApiError::not_found)?;
    }
    let (html, name) = tokio::task::spawn_blocking(move || {
        let html = export::render(&scratchpad, &ids, title.as_deref(), &options)?;
        let title = title.filter(|t| !t.trim().is_empty()).unwrap_or_else(|| {
            if ids.len() == 1 {
                scratchpad
                    .summary(&ids[0])
                    .ok()
                    .flatten()
                    .map(|s| s.subject)
                    .unwrap_or_default()
            } else {
                format!("{} emails", ids.len())
            }
        });
        anyhow::Ok((html, export::file_name(&title)))
    })
    .await
    .map_err(anyhow::Error::from)?
    .map_err(user_error)?;
    Ok((
        [
            (
                header::CONTENT_TYPE,
                HeaderValue::from_static("text/html; charset=utf-8"),
            ),
            (
                header::CONTENT_DISPOSITION,
                content_disposition("attachment", &name),
            ),
            (header::CACHE_CONTROL, HeaderValue::from_static("no-store")),
        ],
        html,
    )
        .into_response())
}

// ---- S3 configs and vault -------------------------------------------------------

async fn overview(State(scratchpad): AppState) -> ApiResult<impl IntoResponse> {
    let configs: Vec<_> = scratchpad
        .s3_configs()
        .iter()
        .map(S3Config::redacted)
        .collect();
    Ok(Json(serde_json::json!({
        "configs": configs,
        "vault": scratchpad.vault_status(),
    })))
}

async fn save_config(
    State(scratchpad): AppState,
    Json(config): Json<S3Config>,
) -> ApiResult<impl IntoResponse> {
    let saved = scratchpad
        .save_s3_config(config)
        .await
        .map_err(user_error)?;
    Ok(Json(saved.redacted()))
}

async fn test_config(
    State(scratchpad): AppState,
    Json(config): Json<S3Config>,
) -> ApiResult<impl IntoResponse> {
    Ok(Json(scratchpad.test_s3_config(config).await))
}

async fn delete_config(
    State(scratchpad): AppState,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    scratchpad.delete_s3_config(&id).await.map_err(user_error)?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct Password {
    password: String,
}

#[derive(Deserialize)]
struct ChangePassword {
    current: String,
    new: String,
}

async fn vault_create(
    State(scratchpad): AppState,
    Json(body): Json<Password>,
) -> ApiResult<StatusCode> {
    scratchpad
        .vault_create(body.password)
        .await
        .map_err(user_error)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn vault_unlock(
    State(scratchpad): AppState,
    Json(body): Json<Password>,
) -> ApiResult<StatusCode> {
    scratchpad
        .vault_unlock(body.password)
        .await
        .map_err(user_error)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn vault_lock(State(scratchpad): AppState) -> StatusCode {
    scratchpad.vault_lock();
    StatusCode::NO_CONTENT
}

async fn vault_password(
    State(scratchpad): AppState,
    Json(body): Json<ChangePassword>,
) -> ApiResult<StatusCode> {
    scratchpad
        .vault_change_password(body.current, body.new)
        .await
        .map_err(user_error)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn vault_sync(State(scratchpad): AppState) -> ApiResult<StatusCode> {
    scratchpad.vault_sync_now().await.map_err(user_error)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn vault_restore(
    State(scratchpad): AppState,
    Json(body): Json<Password>,
) -> ApiResult<impl IntoResponse> {
    let count = scratchpad
        .vault_restore(body.password)
        .await
        .map_err(user_error)?;
    Ok(Json(serde_json::json!({ "configs": count })))
}

async fn vault_reset(State(scratchpad): AppState) -> ApiResult<StatusCode> {
    scratchpad.vault_reset().await.map_err(user_error)?;
    Ok(StatusCode::NO_CONTENT)
}

// ---- cloud shares --------------------------------------------------------------

async fn shares(State(scratchpad): AppState) -> ApiResult<impl IntoResponse> {
    Ok(Json(scratchpad.cloud_shares()?))
}

async fn create_share(
    State(scratchpad): AppState,
    Json(request): Json<CloudShareRequest>,
) -> ApiResult<impl IntoResponse> {
    for id in &request.ids {
        scratchpad.summary(id)?.ok_or_else(ApiError::not_found)?;
    }
    Ok(Json(
        export::share_to_cloud(&scratchpad, request)
            .await
            .map_err(user_error)?,
    ))
}

#[derive(Deserialize)]
struct Renew {
    expires_in_secs: Option<u64>,
}

async fn renew_share(
    State(scratchpad): AppState,
    Path(id): Path<String>,
    Json(body): Json<Renew>,
) -> ApiResult<impl IntoResponse> {
    Ok(Json(
        scratchpad
            .renew_cloud_share(&id, body.expires_in_secs)
            .map_err(user_error)?,
    ))
}

#[derive(Deserialize)]
struct DeleteQuery {
    #[serde(default)]
    keep_object: bool,
}

async fn delete_share(
    State(scratchpad): AppState,
    Path(id): Path<String>,
    Query(q): Query<DeleteQuery>,
) -> ApiResult<StatusCode> {
    scratchpad
        .delete_cloud_share(&id, q.keep_object)
        .await
        .map_err(user_error)?;
    Ok(StatusCode::NO_CONTENT)
}
