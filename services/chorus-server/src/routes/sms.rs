use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, Method, StatusCode};
use axum::response::Response;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use crate::app::AppState;
use crate::auth::api_key::AccountContext;
use crate::db::NewMessage;
use crate::idempotency::{self, IdempotencyAction};
use crate::queue::SendJob;

const ROUTE_PATH: &str = "/v1/sms/send";

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
///
/// Honors the `Idempotency-Key` header: requests with the same key replay
/// the original response without re-queueing the message.
pub async fn send_sms(
    State(state): State<Arc<AppState>>,
    ctx: AccountContext,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let token = match idempotency::begin(
        &state,
        ctx.key_id,
        &headers,
        &Method::POST,
        ROUTE_PATH,
        &body,
    )
    .await
    {
        IdempotencyAction::Skip => None,
        IdempotencyAction::Proceed { token } => Some(token),
        IdempotencyAction::Respond {
            status,
            body: resp_body,
        } => {
            return idempotency::finalize_and_respond(&state, None, status, resp_body).await;
        }
    };

    let req: SendSmsRequest = match serde_json::from_slice(&body) {
        Ok(r) => r,
        Err(e) => {
            let (status, body) = idempotency::bad_request(e.to_string());
            return idempotency::finalize_and_respond(&state, token, status, body).await;
        }
    };

    if let Err(rej) =
        crate::suppression::check_suppression(&state, ctx.account_id, "sms", &req.to).await
    {
        let (status, body) = crate::suppression::rejection_response(rej);
        let bytes = Bytes::from(serde_json::to_vec(&body.0).unwrap_or_default());
        return idempotency::finalize_and_respond(&state, token, status, bytes).await;
    }

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
    let message = match state.message_repo().insert(&new_msg).await {
        Ok(m) => m,
        Err(e) => {
            let (status, body) = idempotency::internal_error(e.to_string());
            return idempotency::finalize_and_respond(&state, token, status, body).await;
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
        let (status, body) = idempotency::internal_error(e.to_string());
        return idempotency::finalize_and_respond(&state, token, status, body).await;
    }

    let response = SendResponse {
        message_id: message.id,
        status: "queued",
    };
    let response_bytes = Bytes::from(serde_json::to_vec(&response).unwrap_or_default());
    idempotency::finalize_and_respond(&state, token, StatusCode::ACCEPTED, response_bytes).await
}
