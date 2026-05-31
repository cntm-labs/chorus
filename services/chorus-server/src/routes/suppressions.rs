use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::app::AppState;
use crate::auth::api_key::AccountContext;
use crate::db::{NewSuppression, Pagination, Suppression};
use crate::suppression::normalize;

use axum::routing::{delete, get};
use axum::Router;

/// Build the suppressions sub-router.
pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/", get(list_suppressions).post(create_suppression))
        .route("/{channel}/{recipient}", delete(delete_suppression))
}

const DEFAULT_LIMIT: i64 = 20;
const MAX_LIMIT: i64 = 100;

/// Query params for `GET /v1/suppressions`.
#[derive(Deserialize)]
pub struct ListParams {
    pub channel: Option<String>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

/// Body for `POST /v1/suppressions`.
#[derive(Deserialize)]
pub struct CreateSuppressionRequest {
    pub channel: String,
    pub recipient: String,
}

/// Wire-form representation of a suppression.
#[derive(Serialize)]
pub struct SuppressionResponse {
    pub channel: String,
    pub recipient: String,
    pub reason: String,
    pub source: String,
    pub created_at: String,
}

/// Paginated list response.
#[derive(Serialize)]
pub struct SuppressionListResponse {
    pub data: Vec<SuppressionResponse>,
    pub limit: i64,
    pub offset: i64,
}

impl From<Suppression> for SuppressionResponse {
    fn from(s: Suppression) -> Self {
        Self {
            channel: s.channel,
            recipient: s.recipient,
            reason: s.reason,
            source: s.source,
            created_at: s.created_at.to_rfc3339(),
        }
    }
}

/// `GET /v1/suppressions`
pub async fn list_suppressions(
    State(state): State<Arc<AppState>>,
    ctx: AccountContext,
    Query(params): Query<ListParams>,
) -> Result<Json<SuppressionListResponse>, (StatusCode, String)> {
    let limit = params.limit.unwrap_or(DEFAULT_LIMIT).min(MAX_LIMIT);
    let offset = params.offset.unwrap_or(0);
    let pagination = Pagination { limit, offset };

    let entries = state
        .suppression_repo()
        .list(ctx.account_id, params.channel.as_deref(), &pagination)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(SuppressionListResponse {
        data: entries.into_iter().map(SuppressionResponse::from).collect(),
        limit,
        offset,
    }))
}

/// `POST /v1/suppressions`
///
/// Returns `201 Created` on a fresh insert; `200 OK` if the entry already
/// existed (idempotent). The response body always echoes the canonical row
/// from the database, including its real `created_at`.
pub async fn create_suppression(
    State(state): State<Arc<AppState>>,
    ctx: AccountContext,
    Json(req): Json<CreateSuppressionRequest>,
) -> Result<(StatusCode, Json<SuppressionResponse>), (StatusCode, String)> {
    let normalized = normalize(&req.channel, &req.recipient)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;

    let entry = NewSuppression {
        account_id: ctx.account_id,
        channel: req.channel,
        recipient: normalized,
        reason: "manual".into(),
        source: "api".into(),
    };

    let result = state
        .suppression_repo()
        .add(&entry)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let status = if result.inserted {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    };
    Ok((status, Json(SuppressionResponse::from(result.entry))))
}

/// `DELETE /v1/suppressions/{channel}/{recipient}`
pub async fn delete_suppression(
    State(state): State<Arc<AppState>>,
    ctx: AccountContext,
    Path((channel, recipient)): Path<(String, String)>,
) -> Result<StatusCode, (StatusCode, String)> {
    let normalized =
        normalize(&channel, &recipient).map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;

    let removed = state
        .suppression_repo()
        .remove(ctx.account_id, &channel, &normalized)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    if removed {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err((StatusCode::NOT_FOUND, "suppression not found".into()))
    }
}
