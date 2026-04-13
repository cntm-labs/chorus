use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use crate::app::AppState;
use crate::auth::api_key::AccountContext;
use crate::db::NewMessage;
use crate::queue::SendJob;

/// SMS send request body.
#[derive(Deserialize)]
pub struct SendSmsRequest {
    /// Recipient phone number in E.164 format.
    pub to: String,
    /// Message body.
    pub body: String,
    /// Optional sender ID or phone number.
    pub from: Option<String>,
}

/// Response returned after queuing a message.
#[derive(Serialize)]
pub struct SendResponse {
    pub message_id: Uuid,
    pub status: &'static str,
}

/// Queue an SMS message for delivery. Returns 202 Accepted.
pub async fn send_sms(
    State(state): State<Arc<AppState>>,
    ctx: AccountContext,
    Json(req): Json<SendSmsRequest>,
) -> Result<(StatusCode, Json<SendResponse>), (StatusCode, String)> {
    let new_msg = NewMessage {
        account_id: ctx.account_id,
        api_key_id: ctx.key_id,
        channel: "sms".into(),
        sender: req.from,
        recipient: req.to,
        subject: None,
        body: req.body,
        environment: ctx.environment,
    };

    let message = state
        .message_repo()
        .insert(&new_msg)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    // Enqueue for background delivery
    let job = SendJob {
        message_id: message.id,
        account_id: message.account_id,
        channel: "sms".into(),
        environment: message.environment.clone(),
        attempt: 0,
    };
    crate::queue::enqueue::enqueue_job(&state, &job)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok((
        StatusCode::ACCEPTED,
        Json(SendResponse {
            message_id: message.id,
            status: "queued",
        }),
    ))
}
