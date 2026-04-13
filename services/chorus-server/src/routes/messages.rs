use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use crate::app::AppState;
use crate::auth::api_key::AccountContext;
use crate::db::{DeliveryEvent, Message, Pagination};

/// Query parameters for message listing.
#[derive(Deserialize)]
pub struct ListParams {
    /// Maximum number of results (default 20, max 100).
    pub limit: Option<i64>,
    /// Offset for pagination.
    pub offset: Option<i64>,
}

const DEFAULT_LIMIT: i64 = 20;
const MAX_LIMIT: i64 = 100;

/// Paginated message list response.
#[derive(Serialize)]
pub struct MessageListResponse {
    pub data: Vec<Message>,
    pub limit: i64,
    pub offset: i64,
}

/// Single message detail with delivery events.
#[derive(Serialize)]
pub struct MessageDetailResponse {
    #[serde(flatten)]
    pub message: Message,
    pub delivery_events: Vec<DeliveryEvent>,
}

/// List messages for the authenticated account.
pub async fn list_messages(
    State(state): State<Arc<AppState>>,
    ctx: AccountContext,
    Query(params): Query<ListParams>,
) -> Result<Json<MessageListResponse>, (StatusCode, String)> {
    let limit = params.limit.unwrap_or(DEFAULT_LIMIT).min(MAX_LIMIT);
    let offset = params.offset.unwrap_or(0);
    let pagination = Pagination { limit, offset };

    let messages = state
        .message_repo()
        .list_by_account(ctx.account_id, &pagination)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(MessageListResponse {
        data: messages,
        limit,
        offset,
    }))
}

/// Get a single message with its delivery events.
pub async fn get_message(
    State(state): State<Arc<AppState>>,
    ctx: AccountContext,
    Path(id): Path<Uuid>,
) -> Result<Json<MessageDetailResponse>, (StatusCode, String)> {
    let message = state
        .message_repo()
        .find_by_id(id, ctx.account_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "message not found".into()))?;

    let delivery_events = state
        .message_repo()
        .get_delivery_events(id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(MessageDetailResponse {
        message,
        delivery_events,
    }))
}
