//! Public, read-only share pages (`/s/{token}`).

use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use scratchpad_core::{Scratchpad, Share};

use crate::api::{self, ApiError, ApiResult, Page, RawQuery};
use crate::assets;

pub fn routes() -> Router<Arc<Scratchpad>> {
    Router::new()
        .route("/s/{token}", get(page))
        .route("/s/{token}/api/share", get(share))
        .route("/s/{token}/api/emails/{id}", get(email))
        .route("/s/{token}/api/emails/{id}/raw", get(raw))
        .route("/s/{token}/api/emails/{id}/html", get(html))
        .route("/s/{token}/api/emails/{id}/view-data", get(view_data))
        .route(
            "/s/{token}/api/emails/{id}/attachments/{index}",
            get(attachment),
        )
}

fn load(scratchpad: &Scratchpad, token: &str) -> ApiResult<Share> {
    scratchpad.share(token)?.ok_or_else(ApiError::not_found)
}

fn authorize(scratchpad: &Scratchpad, token: &str, id: &str) -> ApiResult<()> {
    let share = load(scratchpad, token)?;
    if scratchpad.share_contains(&share, id)? {
        Ok(())
    } else {
        Err(ApiError::not_found())
    }
}

async fn page(
    State(scratchpad): State<Arc<Scratchpad>>,
    Path(token): Path<String>,
) -> ApiResult<Response> {
    load(&scratchpad, &token)?;
    Ok(assets::app_page(&token))
}

async fn share(
    State(scratchpad): State<Arc<Scratchpad>>,
    Path(token): Path<String>,
    Query(page): Query<Page>,
) -> ApiResult<impl IntoResponse> {
    let share = load(&scratchpad, &token)?;
    let emails = scratchpad.share_emails(&share)?;
    Ok(Json(serde_json::json!({
        "title": share.title,
        "mailbox": share.mailbox,
        "created_at": share.created_at,
        "total": emails.len(),
        "emails": page.apply(emails),
    })))
}

async fn email(
    State(scratchpad): State<Arc<Scratchpad>>,
    Path((token, id)): Path<(String, String)>,
) -> ApiResult<impl IntoResponse> {
    authorize(&scratchpad, &token, &id)?;
    Ok(Json(
        scratchpad.email(&id)?.ok_or_else(ApiError::not_found)?,
    ))
}

async fn raw(
    State(scratchpad): State<Arc<Scratchpad>>,
    Path((token, id)): Path<(String, String)>,
    Query(query): Query<RawQuery>,
) -> ApiResult<Response> {
    authorize(&scratchpad, &token, &id)?;
    api::raw_response(&scratchpad, &id, query.download)
}

async fn html(
    State(scratchpad): State<Arc<Scratchpad>>,
    Path((token, id)): Path<(String, String)>,
) -> ApiResult<Response> {
    authorize(&scratchpad, &token, &id)?;
    api::html_response(&scratchpad, &id)
}

async fn view_data(
    State(scratchpad): State<Arc<Scratchpad>>,
    Path((token, id)): Path<(String, String)>,
) -> ApiResult<Response> {
    authorize(&scratchpad, &token, &id)?;
    api::view_data_response(&scratchpad, &id)
}

async fn attachment(
    State(scratchpad): State<Arc<Scratchpad>>,
    Path((token, id, index)): Path<(String, String, usize)>,
) -> ApiResult<Response> {
    authorize(&scratchpad, &token, &id)?;
    api::attachment_response(&scratchpad, &id, index)
}
