use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use serde::Deserialize;
use std::sync::Arc;

use crate::app::AppState;
use crate::auth::api_key::AccountContext;
use crate::db::NewMessage;
use crate::queue::SendJob;
use crate::routes::sms::SendResponse;

/// Email send request body.
#[derive(Deserialize)]
pub struct SendEmailRequest {
    /// Recipient email address.
    pub to: String,
    /// Email subject line.
    pub subject: String,
    /// Email body (HTML or plain text).
    pub body: String,
    /// Optional sender address.
    pub from: Option<String>,
}

/// Queue an email message for delivery. Returns 202 Accepted.
pub async fn send_email(
    State(state): State<Arc<AppState>>,
    ctx: AccountContext,
    Json(req): Json<SendEmailRequest>,
) -> Result<(StatusCode, Json<SendResponse>), (StatusCode, String)> {
    let new_msg = NewMessage {
        account_id: ctx.account_id,
        api_key_id: ctx.key_id,
        channel: "email".into(),
        sender: req.from,
        recipient: req.to,
        subject: Some(req.subject),
        body: req.body,
        environment: ctx.environment,
    };

    let message = state
        .message_repo()
        .insert(&new_msg)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let job = SendJob {
        message_id: message.id,
        account_id: message.account_id,
        channel: "email".into(),
        environment: message.environment.clone(),
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
