use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use rand::Rng;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use crate::app::AppState;
use crate::auth::api_key::AccountContext;
use crate::db::NewWebhook;

/// Request body for registering a webhook.
#[derive(Deserialize)]
pub struct CreateWebhookRequest {
    /// Callback URL to receive events.
    pub url: String,
    /// Event types to subscribe to.
    pub events: Vec<String>,
}

/// Response after creating a webhook.
#[derive(Serialize)]
pub struct WebhookResponse {
    pub id: Uuid,
    pub url: String,
    pub secret: String,
    pub events: Vec<String>,
    pub created_at: String,
}

/// Response for listing webhooks (secret redacted).
#[derive(Serialize)]
pub struct WebhookListItem {
    pub id: Uuid,
    pub url: String,
    pub events: Vec<String>,
    pub created_at: String,
}

const VALID_EVENTS: &[&str] = &[
    "message.queued",
    "message.sent",
    "message.delivered",
    "message.failed",
];

/// Register a new webhook.
pub async fn create_webhook(
    State(state): State<Arc<AppState>>,
    ctx: AccountContext,
    Json(req): Json<CreateWebhookRequest>,
) -> Result<(StatusCode, Json<WebhookResponse>), (StatusCode, String)> {
    for event in &req.events {
        if !VALID_EVENTS.contains(&event.as_str()) {
            return Err((
                StatusCode::BAD_REQUEST,
                format!("invalid event type: {event}"),
            ));
        }
    }

    let secret: String = hex::encode(rand::thread_rng().gen::<[u8; 32]>());

    let webhook = NewWebhook {
        account_id: ctx.account_id,
        url: req.url,
        secret: secret.clone(),
        events: req.events,
    };

    let created = state
        .webhook_repo()
        .insert(&webhook)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok((
        StatusCode::CREATED,
        Json(WebhookResponse {
            id: created.id,
            url: created.url,
            secret,
            events: created.events,
            created_at: created.created_at.to_rfc3339(),
        }),
    ))
}

/// List all active webhooks for the account.
pub async fn list_webhooks(
    State(state): State<Arc<AppState>>,
    ctx: AccountContext,
) -> Result<Json<Vec<WebhookListItem>>, (StatusCode, String)> {
    let webhooks = state
        .webhook_repo()
        .list_by_account(ctx.account_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let items: Vec<WebhookListItem> = webhooks
        .into_iter()
        .map(|w| WebhookListItem {
            id: w.id,
            url: w.url,
            events: w.events,
            created_at: w.created_at.to_rfc3339(),
        })
        .collect();

    Ok(Json(items))
}

/// Delete a webhook.
pub async fn delete_webhook(
    State(state): State<Arc<AppState>>,
    ctx: AccountContext,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, (StatusCode, String)> {
    state
        .webhook_repo()
        .delete(id, ctx.account_id)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;

    Ok(StatusCode::NO_CONTENT)
}
