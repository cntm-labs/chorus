use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use crate::app::AppState;
use crate::auth::admin::AdminContext;
use crate::db::{DeliveryEvent, Message};
use crate::queue::{SendJob, DEAD_LETTER_KEY, QUEUE_KEY};

/// DLQ message summary combining Redis job data with DB message data.
#[derive(Serialize)]
pub struct DlqMessage {
    pub message_id: Uuid,
    pub account_id: Uuid,
    pub channel: String,
    pub recipient: String,
    pub status: String,
    pub attempts: i32,
    pub error_message: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// DLQ message detail with retry history.
#[derive(Serialize)]
pub struct DlqMessageDetail {
    pub message: Message,
    pub delivery_events: Vec<DeliveryEvent>,
}

/// Query parameters for listing DLQ messages.
#[derive(Deserialize)]
pub struct DlqListParams {
    pub limit: Option<i64>,
    pub offset: Option<i64>,
    pub channel: Option<String>,
    pub account_id: Option<Uuid>,
}

/// `GET /admin/dlq` — list DLQ messages with optional filters.
pub async fn list(
    State(state): State<Arc<AppState>>,
    _admin: AdminContext,
    Query(params): Query<DlqListParams>,
) -> Result<Json<Vec<DlqMessage>>, (StatusCode, String)> {
    let limit = params.limit.unwrap_or(50);
    let offset = params.offset.unwrap_or(0);

    // Read jobs from Redis DLQ
    let mut conn = state
        .redis
        .get_multiplexed_tokio_connection()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let raw_jobs: Vec<String> = redis::cmd("LRANGE")
        .arg(DEAD_LETTER_KEY)
        .arg(offset)
        .arg(offset + limit - 1)
        .query_async(&mut conn)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let mut results = Vec::new();

    for raw in &raw_jobs {
        let job: SendJob = match serde_json::from_str(raw) {
            Ok(j) => j,
            Err(_) => continue,
        };

        // Apply filters
        if let Some(ref ch) = params.channel {
            if &job.channel != ch {
                continue;
            }
        }
        if let Some(aid) = params.account_id {
            if job.account_id != aid {
                continue;
            }
        }

        // Look up message details from DB (no account_id scoping — admin query)
        if let Ok(Some(msg)) = state.admin_repo().get_message_by_id(job.message_id).await {
            results.push(DlqMessage {
                message_id: msg.id,
                account_id: msg.account_id,
                channel: msg.channel.clone(),
                recipient: msg.recipient.clone(),
                status: msg.status.clone(),
                attempts: msg.attempts,
                error_message: msg.error_message.clone(),
                created_at: msg.created_at,
            });
        }
    }

    Ok(Json(results))
}

/// `GET /admin/dlq/{message_id}` — DLQ message detail with retry history.
pub async fn detail(
    State(state): State<Arc<AppState>>,
    _admin: AdminContext,
    Path(message_id): Path<Uuid>,
) -> Result<Json<DlqMessageDetail>, (StatusCode, String)> {
    let message = state
        .admin_repo()
        .get_message_by_id(message_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "message not found".into()))?;

    let delivery_events = state
        .message_repo()
        .get_delivery_events(message_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(DlqMessageDetail {
        message,
        delivery_events,
    }))
}

/// `POST /admin/dlq/{message_id}/retry` — re-enqueue a single message.
pub async fn retry_single(
    State(state): State<Arc<AppState>>,
    _admin: AdminContext,
    Path(message_id): Path<Uuid>,
) -> Result<StatusCode, (StatusCode, String)> {
    let mut conn = state
        .redis
        .get_multiplexed_tokio_connection()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    // Find and remove from DLQ
    let job = find_and_remove_from_dlq(&mut conn, message_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "message not in DLQ".into()))?;

    // Re-enqueue with reset attempt counter
    let retry_job = SendJob { attempt: 0, ..job };
    let payload = serde_json::to_string(&retry_job)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    redis::cmd("LPUSH")
        .arg(QUEUE_KEY)
        .arg(payload)
        .query_async::<i64>(&mut conn)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(StatusCode::NO_CONTENT)
}

/// Request body for batch retry.
#[derive(Deserialize)]
pub struct RetryBatchRequest {
    pub message_ids: Vec<Uuid>,
}

/// Response for batch retry.
#[derive(Serialize)]
pub struct RetryBatchResponse {
    pub retried: usize,
    pub not_found: usize,
}

/// `POST /admin/dlq/retry-batch` — re-enqueue multiple messages.
pub async fn retry_batch(
    State(state): State<Arc<AppState>>,
    _admin: AdminContext,
    Json(body): Json<RetryBatchRequest>,
) -> Result<Json<RetryBatchResponse>, (StatusCode, String)> {
    let mut conn = state
        .redis
        .get_multiplexed_tokio_connection()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let mut retried = 0;
    let mut not_found = 0;

    for mid in &body.message_ids {
        match find_and_remove_from_dlq(&mut conn, *mid).await {
            Ok(Some(job)) => {
                let retry_job = SendJob { attempt: 0, ..job };
                let payload = serde_json::to_string(&retry_job)
                    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
                redis::cmd("LPUSH")
                    .arg(QUEUE_KEY)
                    .arg(payload)
                    .query_async::<i64>(&mut conn)
                    .await
                    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
                retried += 1;
            }
            Ok(None) => not_found += 1,
            Err(e) => return Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
        }
    }

    Ok(Json(RetryBatchResponse { retried, not_found }))
}

/// `DELETE /admin/dlq/{message_id}` — purge a single message from DLQ.
pub async fn purge_single(
    State(state): State<Arc<AppState>>,
    _admin: AdminContext,
    Path(message_id): Path<Uuid>,
) -> Result<StatusCode, (StatusCode, String)> {
    let mut conn = state
        .redis
        .get_multiplexed_tokio_connection()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let removed = find_and_remove_from_dlq(&mut conn, message_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    if removed.is_none() {
        return Err((StatusCode::NOT_FOUND, "message not in DLQ".into()));
    }

    Ok(StatusCode::NO_CONTENT)
}

/// Query parameters for purge all.
#[derive(Deserialize)]
pub struct PurgeParams {
    pub older_than_days: Option<i64>,
}

/// Response for purge all.
#[derive(Serialize)]
pub struct PurgeResponse {
    pub purged: i64,
}

/// `DELETE /admin/dlq/purge` — purge old DLQ messages.
pub async fn purge_all(
    State(state): State<Arc<AppState>>,
    _admin: AdminContext,
    Query(params): Query<PurgeParams>,
) -> Result<Json<PurgeResponse>, (StatusCode, String)> {
    let mut conn = state
        .redis
        .get_multiplexed_tokio_connection()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    if let Some(days) = params.older_than_days {
        // Selective purge: scan DLQ and remove entries older than N days
        let cutoff = chrono::Utc::now() - chrono::Duration::days(days);
        let all_jobs: Vec<String> = redis::cmd("LRANGE")
            .arg(DEAD_LETTER_KEY)
            .arg(0)
            .arg(-1)
            .query_async(&mut conn)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

        let mut purged: i64 = 0;
        for raw in &all_jobs {
            let job: SendJob = match serde_json::from_str(raw) {
                Ok(j) => j,
                Err(_) => continue,
            };
            // Check message created_at from DB
            if let Ok(Some(msg)) = state.admin_repo().get_message_by_id(job.message_id).await {
                if msg.created_at < cutoff {
                    let _: i64 = redis::cmd("LREM")
                        .arg(DEAD_LETTER_KEY)
                        .arg(1)
                        .arg(raw)
                        .query_async(&mut conn)
                        .await
                        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
                    purged += 1;
                }
            }
        }
        Ok(Json(PurgeResponse { purged }))
    } else {
        // Purge all: get length then DEL
        let len: i64 = redis::cmd("LLEN")
            .arg(DEAD_LETTER_KEY)
            .query_async(&mut conn)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

        let _: i64 = redis::cmd("DEL")
            .arg(DEAD_LETTER_KEY)
            .query_async(&mut conn)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

        Ok(Json(PurgeResponse { purged: len }))
    }
}

/// Find a job in the DLQ by message_id and remove it.
async fn find_and_remove_from_dlq(
    conn: &mut redis::aio::MultiplexedConnection,
    message_id: Uuid,
) -> anyhow::Result<Option<SendJob>> {
    let all_jobs: Vec<String> = redis::cmd("LRANGE")
        .arg(DEAD_LETTER_KEY)
        .arg(0)
        .arg(-1)
        .query_async(conn)
        .await?;

    for raw in &all_jobs {
        let job: SendJob = match serde_json::from_str(raw) {
            Ok(j) => j,
            Err(_) => continue,
        };
        if job.message_id == message_id {
            redis::cmd("LREM")
                .arg(DEAD_LETTER_KEY)
                .arg(1)
                .arg(raw)
                .query_async::<i64>(conn)
                .await?;
            return Ok(Some(job));
        }
    }

    Ok(None)
}
