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

/// Maximum recipients per batch request.
const MAX_BATCH_SIZE: usize = 100;

/// A single SMS recipient in a batch.
#[derive(Deserialize)]
pub struct SmsBatchRecipient {
    /// Recipient phone number in E.164 format.
    pub to: String,
    /// Message body.
    pub body: String,
}

/// SMS batch send request.
#[derive(Deserialize)]
pub struct SendSmsBatchRequest {
    /// Optional sender ID or phone number.
    pub from: Option<String>,
    /// List of recipients with individual message bodies.
    pub recipients: Vec<SmsBatchRecipient>,
}

/// A single email recipient in a batch.
#[derive(Deserialize)]
pub struct EmailBatchRecipient {
    /// Recipient email address.
    pub to: String,
    /// Email subject line.
    pub subject: String,
    /// Email body (HTML or plain text).
    pub body: String,
}

/// Email batch send request.
#[derive(Deserialize)]
pub struct SendEmailBatchRequest {
    /// Optional sender address.
    pub from: Option<String>,
    /// List of recipients with individual subjects and bodies.
    pub recipients: Vec<EmailBatchRecipient>,
}

/// One message result in the batch response.
#[derive(Serialize)]
pub struct BatchMessageResult {
    pub message_id: Uuid,
    pub to: String,
    pub status: &'static str,
}

/// Batch send response. Includes partial results if an error occurred mid-batch.
#[derive(Serialize)]
pub struct BatchSendResponse {
    pub messages: Vec<BatchMessageResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Queue a batch of SMS messages. Returns 202 Accepted.
///
/// On partial failure, returns already-queued messages with an error field.
pub async fn send_sms_batch(
    State(state): State<Arc<AppState>>,
    ctx: AccountContext,
    Json(req): Json<SendSmsBatchRequest>,
) -> Result<(StatusCode, Json<BatchSendResponse>), (StatusCode, String)> {
    validate_batch_size(req.recipients.len())?;

    let mut results = Vec::with_capacity(req.recipients.len());

    for recipient in &req.recipients {
        let new_msg = NewMessage {
            account_id: ctx.account_id,
            api_key_id: ctx.key_id,
            channel: "sms".into(),
            sender: req.from.clone(),
            recipient: recipient.to.clone(),
            subject: None,
            body: recipient.body.clone(),
            environment: ctx.environment.clone(),
        };

        let message = match state.message_repo().insert(&new_msg).await {
            Ok(m) => m,
            Err(e) => {
                return Ok((
                    StatusCode::ACCEPTED,
                    Json(BatchSendResponse {
                        messages: results,
                        error: Some(format!("failed at recipient {}: {}", recipient.to, e)),
                    }),
                ));
            }
        };

        let job = SendJob {
            message_id: message.id,
            account_id: message.account_id,
            channel: "sms".into(),
            environment: message.environment.clone(),
            attempt: 0,
        };
        if let Err(e) = crate::queue::enqueue::notify(&state, &job).await {
            return Ok((
                StatusCode::ACCEPTED,
                Json(BatchSendResponse {
                    messages: results,
                    error: Some(format!("failed to enqueue for {}: {}", recipient.to, e)),
                }),
            ));
        }

        results.push(BatchMessageResult {
            message_id: message.id,
            to: recipient.to.clone(),
            status: "queued",
        });
    }

    Ok((
        StatusCode::ACCEPTED,
        Json(BatchSendResponse {
            messages: results,
            error: None,
        }),
    ))
}

/// Queue a batch of email messages. Returns 202 Accepted.
///
/// On partial failure, returns already-queued messages with an error field.
pub async fn send_email_batch(
    State(state): State<Arc<AppState>>,
    ctx: AccountContext,
    Json(req): Json<SendEmailBatchRequest>,
) -> Result<(StatusCode, Json<BatchSendResponse>), (StatusCode, String)> {
    validate_batch_size(req.recipients.len())?;

    let mut results = Vec::with_capacity(req.recipients.len());

    for recipient in &req.recipients {
        let new_msg = NewMessage {
            account_id: ctx.account_id,
            api_key_id: ctx.key_id,
            channel: "email".into(),
            sender: req.from.clone(),
            recipient: recipient.to.clone(),
            subject: Some(recipient.subject.clone()),
            body: recipient.body.clone(),
            environment: ctx.environment.clone(),
        };

        let message = match state.message_repo().insert(&new_msg).await {
            Ok(m) => m,
            Err(e) => {
                return Ok((
                    StatusCode::ACCEPTED,
                    Json(BatchSendResponse {
                        messages: results,
                        error: Some(format!("failed at recipient {}: {}", recipient.to, e)),
                    }),
                ));
            }
        };

        let job = SendJob {
            message_id: message.id,
            account_id: message.account_id,
            channel: "email".into(),
            environment: message.environment.clone(),
            attempt: 0,
        };
        if let Err(e) = crate::queue::enqueue::notify(&state, &job).await {
            return Ok((
                StatusCode::ACCEPTED,
                Json(BatchSendResponse {
                    messages: results,
                    error: Some(format!("failed to enqueue for {}: {}", recipient.to, e)),
                }),
            ));
        }

        results.push(BatchMessageResult {
            message_id: message.id,
            to: recipient.to.clone(),
            status: "queued",
        });
    }

    Ok((
        StatusCode::ACCEPTED,
        Json(BatchSendResponse {
            messages: results,
            error: None,
        }),
    ))
}

/// Validate that the batch size is within bounds.
fn validate_batch_size(size: usize) -> Result<(), (StatusCode, String)> {
    if size == 0 {
        return Err((StatusCode::BAD_REQUEST, "recipients cannot be empty".into()));
    }
    if size > MAX_BATCH_SIZE {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("max {MAX_BATCH_SIZE} recipients per batch"),
        ));
    }
    Ok(())
}
