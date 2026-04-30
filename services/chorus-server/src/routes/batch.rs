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
    /// `Some` only when `status == "queued"`.
    pub message_id: Option<Uuid>,
    pub to: String,
    /// `"queued"` (insert + enqueue both succeeded), `"suppressed"`,
    /// `"invalid"` (recipient failed format validation, e.g. non-E.164 SMS),
    /// `"failed"` (this entry's insert or enqueue failed), or
    /// `"skipped"` (an earlier entry failed; this one was never attempted).
    pub status: &'static str,
    /// Present when `status == "suppressed"` (suppression reason)
    /// or `status == "invalid"` (validation error description).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
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
/// Suppressed recipients are filtered out in a first pass before any enqueue
/// operations. Returns 207 Multi-Status when at least one recipient is
/// suppressed; 202 Accepted when all entries are queued.
/// On partial enqueue failure, returns already-queued results with an error field.
pub async fn send_sms_batch(
    State(state): State<Arc<AppState>>,
    ctx: AccountContext,
    Json(req): Json<SendSmsBatchRequest>,
) -> Result<(StatusCode, Json<BatchSendResponse>), (StatusCode, String)> {
    validate_batch_size(req.recipients.len())?;

    // --- Pass 1: suppression check for all recipients ---
    let mut results: Vec<BatchMessageResult> = Vec::with_capacity(req.recipients.len());

    for recipient in &req.recipients {
        match crate::suppression::check_suppression(&state, ctx.account_id, "sms", &recipient.to)
            .await
        {
            Err(crate::suppression::SuppressionRejection::Suppressed { reason }) => {
                results.push(BatchMessageResult {
                    message_id: None,
                    to: recipient.to.clone(),
                    status: "suppressed",
                    reason: Some(reason),
                });
            }
            Err(crate::suppression::SuppressionRejection::InvalidRecipient) => {
                results.push(BatchMessageResult {
                    message_id: None,
                    to: recipient.to.clone(),
                    status: "invalid",
                    reason: Some("not a valid E.164 phone number".into()),
                });
            }
            Err(crate::suppression::SuppressionRejection::Db(e)) => {
                return Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string()));
            }
            Ok(()) => {
                // Placeholder — Pass 2 transitions to "queued" / "failed" / "skipped".
                results.push(BatchMessageResult {
                    message_id: None,
                    to: recipient.to.clone(),
                    status: "pending",
                    reason: None,
                });
            }
        }
    }

    // --- Pass 2: insert + enqueue non-suppressed recipients ---
    let mut enqueue_error: Option<String> = None;

    for (i, recipient) in req.recipients.iter().enumerate() {
        if results[i].status != "pending" {
            continue;
        }
        if enqueue_error.is_some() {
            // An earlier entry failed; don't attempt this one.
            results[i].status = "skipped";
            continue;
        }

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
                results[i].status = "failed";
                enqueue_error = Some(format!("failed at recipient {}: {}", recipient.to, e));
                continue;
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
            results[i].status = "failed";
            enqueue_error = Some(format!("failed to enqueue for {}: {}", recipient.to, e));
            continue;
        }

        results[i].message_id = Some(message.id);
        results[i].status = "queued";
    }

    // 207 when any entry has a non-queued outcome (suppressed/invalid/failed/skipped),
    // 202 only when all entries successfully queued.
    let all_queued = results.iter().all(|r| r.status == "queued");
    let status = if all_queued {
        StatusCode::ACCEPTED
    } else {
        StatusCode::MULTI_STATUS
    };
    Ok((
        status,
        Json(BatchSendResponse {
            messages: results,
            error: enqueue_error,
        }),
    ))
}

/// Queue a batch of email messages. Returns 202 Accepted.
///
/// Suppressed recipients are filtered out in a first pass before any enqueue
/// operations. Returns 207 Multi-Status when at least one recipient is
/// suppressed; 202 Accepted when all entries are queued.
/// On partial enqueue failure, returns already-queued results with an error field.
pub async fn send_email_batch(
    State(state): State<Arc<AppState>>,
    ctx: AccountContext,
    Json(req): Json<SendEmailBatchRequest>,
) -> Result<(StatusCode, Json<BatchSendResponse>), (StatusCode, String)> {
    validate_batch_size(req.recipients.len())?;

    // --- Pass 1: suppression check for all recipients ---
    let mut results: Vec<BatchMessageResult> = Vec::with_capacity(req.recipients.len());

    for recipient in &req.recipients {
        match crate::suppression::check_suppression(&state, ctx.account_id, "email", &recipient.to)
            .await
        {
            Err(crate::suppression::SuppressionRejection::Suppressed { reason }) => {
                results.push(BatchMessageResult {
                    message_id: None,
                    to: recipient.to.clone(),
                    status: "suppressed",
                    reason: Some(reason),
                });
            }
            Err(crate::suppression::SuppressionRejection::InvalidRecipient) => {
                results.push(BatchMessageResult {
                    message_id: None,
                    to: recipient.to.clone(),
                    status: "invalid",
                    reason: Some("invalid email address".into()),
                });
            }
            Err(crate::suppression::SuppressionRejection::Db(e)) => {
                return Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string()));
            }
            Ok(()) => {
                // Placeholder — Pass 2 transitions to "queued" / "failed" / "skipped".
                results.push(BatchMessageResult {
                    message_id: None,
                    to: recipient.to.clone(),
                    status: "pending",
                    reason: None,
                });
            }
        }
    }

    // --- Pass 2: insert + enqueue non-suppressed recipients ---
    let mut enqueue_error: Option<String> = None;

    for (i, recipient) in req.recipients.iter().enumerate() {
        if results[i].status != "pending" {
            continue;
        }
        if enqueue_error.is_some() {
            // An earlier entry failed; don't attempt this one.
            results[i].status = "skipped";
            continue;
        }

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
                results[i].status = "failed";
                enqueue_error = Some(format!("failed at recipient {}: {}", recipient.to, e));
                continue;
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
            results[i].status = "failed";
            enqueue_error = Some(format!("failed to enqueue for {}: {}", recipient.to, e));
            continue;
        }

        results[i].message_id = Some(message.id);
        results[i].status = "queued";
    }

    // 207 when any entry has a non-queued outcome (suppressed/invalid/failed/skipped),
    // 202 only when all entries successfully queued.
    let all_queued = results.iter().all(|r| r.status == "queued");
    let status = if all_queued {
        StatusCode::ACCEPTED
    } else {
        StatusCode::MULTI_STATUS
    };
    Ok((
        status,
        Json(BatchSendResponse {
            messages: results,
            error: enqueue_error,
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
