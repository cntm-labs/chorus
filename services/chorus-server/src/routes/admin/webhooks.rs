use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use crate::app::AppState;
use crate::auth::admin::AdminContext;

/// Webhook summary for admin list view.
#[derive(Serialize, sqlx::FromRow)]
pub struct AdminWebhook {
    pub id: Uuid,
    pub account_id: Uuid,
    pub url: String,
    pub events: Vec<String>,
    pub is_active: bool,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// Webhook delivery log entry.
#[derive(Serialize, sqlx::FromRow)]
pub struct WebhookDelivery {
    pub id: Uuid,
    pub webhook_id: Uuid,
    pub event: String,
    pub payload: serde_json::Value,
    pub response_status: Option<i32>,
    pub response_body: Option<String>,
    pub attempt: i32,
    pub success: bool,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// Pagination params for delivery log.
#[derive(Deserialize)]
pub struct DeliveryParams {
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

/// Request body for updating webhook status.
#[derive(Deserialize)]
pub struct UpdateWebhookRequest {
    pub is_active: bool,
}

/// Response for bulk disable.
#[derive(Serialize)]
pub struct BulkDisableResponse {
    pub affected: u64,
}

/// `GET /admin/webhooks` — list all webhooks across accounts.
pub async fn list_all(
    State(state): State<Arc<AppState>>,
    _admin: AdminContext,
) -> Result<Json<Vec<AdminWebhook>>, (StatusCode, String)> {
    let webhooks = state
        .admin_repo()
        .list_all_webhooks()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(webhooks))
}

/// `GET /admin/webhooks/{id}/deliveries` — webhook delivery log.
pub async fn deliveries(
    State(state): State<Arc<AppState>>,
    _admin: AdminContext,
    Path(id): Path<Uuid>,
    Query(params): Query<DeliveryParams>,
) -> Result<Json<Vec<WebhookDelivery>>, (StatusCode, String)> {
    let limit = params.limit.unwrap_or(50);
    let offset = params.offset.unwrap_or(0);

    let deliveries = state
        .admin_repo()
        .get_webhook_deliveries(id, limit, offset)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(deliveries))
}

/// `POST /admin/webhooks/{id}/test` — send a test event to webhook.
pub async fn test_webhook(
    State(state): State<Arc<AppState>>,
    _admin: AdminContext,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, (StatusCode, String)> {
    // Look up webhook
    let webhook = state
        .admin_repo()
        .get_webhook_by_id(id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "webhook not found".into()))?;

    // Build test payload
    let test_payload = crate::queue::webhook_dispatch::WebhookPayload {
        event: "test.ping".into(),
        message_id: Uuid::nil(),
        channel: "test".into(),
        provider: None,
        status: "test".into(),
        timestamp: chrono::Utc::now().to_rfc3339(),
    };

    let body = serde_json::to_string(&test_payload)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    // Deliver using HTTP client directly
    let client = state.http_client();
    let signature = {
        use hmac::{Hmac, Mac};
        use sha2::Sha256;
        let mut mac = Hmac::<Sha256>::new_from_slice(webhook.secret.as_bytes())
            .expect("HMAC accepts any key length");
        mac.update(body.as_bytes());
        hex::encode(mac.finalize().into_bytes())
    };

    let result = client
        .post(&webhook.url)
        .header("Content-Type", "application/json")
        .header("X-Chorus-Signature", &signature)
        .header("X-Chorus-Event", "test.ping")
        .header(
            "X-Chorus-Timestamp",
            &chrono::Utc::now().timestamp().to_string(),
        )
        .body(body)
        .send()
        .await;

    match result {
        Ok(resp) if resp.status().is_success() => Ok(StatusCode::NO_CONTENT),
        Ok(resp) => Err((
            StatusCode::BAD_GATEWAY,
            format!("webhook returned {}", resp.status()),
        )),
        Err(e) => Err((StatusCode::BAD_GATEWAY, format!("webhook error: {e}"))),
    }
}

/// `PATCH /admin/webhooks/{id}` — enable/disable a webhook.
pub async fn update_status(
    State(state): State<Arc<AppState>>,
    _admin: AdminContext,
    Path(id): Path<Uuid>,
    Json(body): Json<UpdateWebhookRequest>,
) -> Result<StatusCode, (StatusCode, String)> {
    state
        .admin_repo()
        .update_webhook_status(id, body.is_active)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(StatusCode::NO_CONTENT)
}

/// `POST /admin/webhooks/disable-account/{account_id}` — disable all webhooks for an account.
pub async fn disable_account(
    State(state): State<Arc<AppState>>,
    _admin: AdminContext,
    Path(account_id): Path<Uuid>,
) -> Result<Json<BulkDisableResponse>, (StatusCode, String)> {
    let affected = state
        .admin_repo()
        .disable_account_webhooks(account_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(BulkDisableResponse { affected }))
}
