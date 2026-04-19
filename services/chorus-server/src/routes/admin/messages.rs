use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use crate::app::AppState;
use crate::auth::admin::AdminContext;
use crate::db::{DeliveryEvent, Message};

/// Filters for cross-account message search.
#[derive(Deserialize)]
pub struct MessageSearchFilters {
    pub account_id: Option<Uuid>,
    pub channel: Option<String>,
    pub status: Option<String>,
    pub provider: Option<String>,
    pub date_from: Option<chrono::DateTime<chrono::Utc>>,
    pub date_to: Option<chrono::DateTime<chrono::Utc>>,
    pub recipient: Option<String>,
    pub min_cost: Option<i64>,
    pub max_cost: Option<i64>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

/// Message detail with delivery timeline.
#[derive(Serialize)]
pub struct MessageDetail {
    pub message: Message,
    pub delivery_events: Vec<DeliveryEvent>,
}

/// `GET /admin/messages` — search messages across all accounts.
pub async fn search(
    State(state): State<Arc<AppState>>,
    _admin: AdminContext,
    Query(filters): Query<MessageSearchFilters>,
) -> Result<Json<Vec<Message>>, (StatusCode, String)> {
    let messages = state
        .admin_repo()
        .search_messages(&filters)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(messages))
}

/// `GET /admin/messages/{id}` — message detail with delivery timeline.
pub async fn detail(
    State(state): State<Arc<AppState>>,
    _admin: AdminContext,
    Path(id): Path<Uuid>,
) -> Result<Json<MessageDetail>, (StatusCode, String)> {
    let message = state
        .admin_repo()
        .get_message_by_id(id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "message not found".into()))?;

    let delivery_events = state
        .message_repo()
        .get_delivery_events(id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(MessageDetail {
        message,
        delivery_events,
    }))
}
