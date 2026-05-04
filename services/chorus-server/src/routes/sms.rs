use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use crate::app::AppState;
use crate::auth::api_key::AccountContext;
use crate::db::NewMessage;
use crate::idempotency::{self, IdempotencyAction, IdempotencyToken};
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
        } => return (status, resp_body).into_response(),
    };

    let req: SendSmsRequest = match serde_json::from_slice(&body) {
        Ok(r) => r,
        Err(e) => return finalize_and_respond(&state, token, error_400(e.to_string())).await,
    };

    if let Err(rej) =
        crate::suppression::check_suppression(&state, ctx.account_id, "sms", &req.to).await
    {
        let (status, body) = crate::suppression::rejection_response(rej);
        let bytes = Bytes::from(serde_json::to_vec(&body.0).unwrap_or_default());
        return finalize_and_respond(&state, token, (status, bytes)).await;
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
        Err(e) => return finalize_and_respond(&state, token, error_500(e.to_string())).await,
    };

    let job = SendJob {
        message_id: message.id,
        account_id: message.account_id,
        channel: "sms".into(),
        environment: message.environment.clone(),
        attempt: 0,
    };
    if let Err(e) = crate::queue::enqueue::notify(&state, &job).await {
        return finalize_and_respond(&state, token, error_500(e.to_string())).await;
    }

    let response = SendResponse {
        message_id: message.id,
        status: "queued",
    };
    let response_bytes = Bytes::from(serde_json::to_vec(&response).unwrap_or_default());
    finalize_and_respond(&state, token, (StatusCode::ACCEPTED, response_bytes)).await
}

/// Cache the response (if a token is held) and turn it into an axum Response.
async fn finalize_and_respond(
    state: &Arc<AppState>,
    token: Option<IdempotencyToken>,
    (status, body): (StatusCode, Bytes),
) -> Response {
    if let Some(t) = token {
        idempotency::finalize(state, t, status, &body).await;
    }
    (status, body).into_response()
}

fn error_400(msg: String) -> (StatusCode, Bytes) {
    (
        StatusCode::BAD_REQUEST,
        Bytes::from(
            serde_json::to_vec(
                &serde_json::json!({ "error": { "code": "bad_request", "message": msg } }),
            )
            .unwrap_or_default(),
        ),
    )
}

fn error_500(msg: String) -> (StatusCode, Bytes) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Bytes::from(
            serde_json::to_vec(
                &serde_json::json!({ "error": { "code": "internal", "message": msg } }),
            )
            .unwrap_or_default(),
        ),
    )
}
