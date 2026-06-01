use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, Method, StatusCode};
use axum::response::Response;
use axum::Json;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use crate::app::AppState;
use crate::auth::api_key::AccountContext;
use crate::idempotency::{self, IdempotencyAction};

use axum::routing::post;
use axum::Router;

const ROUTE_PATH: &str = "/v1/otp/send";

/// Build the OTP sub-router.
pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/send", post(send_otp))
        .route("/verify", post(verify_otp))
}

/// OTP send request body.
#[derive(Deserialize)]
pub struct SendOtpRequest {
    /// Recipient phone number or email address.
    pub to: String,
    /// Optional application name shown in the OTP message.
    pub app_name: Option<String>,
}

/// OTP send response.
#[derive(Serialize)]
pub struct SendOtpResponse {
    pub otp_id: Uuid,
}

/// OTP verify request body.
#[derive(Deserialize)]
pub struct VerifyOtpRequest {
    /// The OTP ID returned from the send endpoint.
    pub otp_id: Uuid,
    /// The 6-digit code entered by the user.
    pub code: String,
}

/// OTP verify response.
#[derive(Serialize)]
pub struct VerifyOtpResponse {
    pub verified: bool,
}

/// Send a one-time password to the recipient.
///
/// Honors the `Idempotency-Key` header. Note that OTP also has a built-in
/// per-recipient dedupe at the Redis layer; idempotency adds replay-safe
/// retry semantics on top.
pub async fn send_otp(
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
        } => return idempotency::finalize_and_respond(&state, None, status, resp_body).await,
    };

    let req: SendOtpRequest = match serde_json::from_slice(&body) {
        Ok(r) => r,
        Err(e) => {
            let (status, body) = idempotency::bad_request(e.to_string());
            return idempotency::finalize_and_respond(&state, token, status, body).await;
        }
    };

    let channel = if req.to.contains('@') { "email" } else { "sms" };
    let code = crate::otp::generate_code();

    if let Err(rej) =
        crate::suppression::check_suppression(&state, ctx.account_id, channel, &req.to).await
    {
        let (status, body) = crate::suppression::rejection_response(rej);
        let bytes = Bytes::from(serde_json::to_vec(&body.0).unwrap_or_default());
        return idempotency::finalize_and_respond(&state, token, status, bytes).await;
    }

    let otp_id = match crate::otp::store(&state.redis, &req.to, &code).await {
        Ok(id) => id,
        Err(e) => {
            let (status, body) = idempotency::internal_error(e.to_string());
            return idempotency::finalize_and_respond(&state, token, status, body).await;
        }
    };

    let app_name = req.app_name.as_deref().unwrap_or("Chorus");
    let message_body = format!("Your {app_name} verification code is: {code}");

    let new_msg = crate::db::NewMessage {
        account_id: ctx.account_id,
        api_key_id: ctx.key_id,
        channel: channel.into(),
        sender: None,
        recipient: req.to,
        subject: Some(format!("{app_name} verification code")),
        body: message_body,
        environment: ctx.environment.clone(),
    };

    let message = match state.message_repo().insert(&new_msg).await {
        Ok(m) => m,
        Err(e) => {
            let (status, body) = idempotency::internal_error(e.to_string());
            return idempotency::finalize_and_respond(&state, token, status, body).await;
        }
    };

    let job = crate::queue::SendJob {
        message_id: message.id,
        account_id: message.account_id,
        channel: message.channel.clone(),
        environment: ctx.environment,
        attempt: 0,
    };
    if let Err(e) = crate::queue::enqueue::notify(&state, &job).await {
        let (status, body) = idempotency::internal_error(e.to_string());
        return idempotency::finalize_and_respond(&state, token, status, body).await;
    }

    let response = SendOtpResponse { otp_id };
    let response_bytes = Bytes::from(serde_json::to_vec(&response).unwrap_or_default());
    idempotency::finalize_and_respond(&state, token, StatusCode::CREATED, response_bytes).await
}

/// Verify a one-time password.
pub async fn verify_otp(
    State(state): State<Arc<AppState>>,
    _ctx: AccountContext,
    Json(req): Json<VerifyOtpRequest>,
) -> Result<Json<VerifyOtpResponse>, (StatusCode, String)> {
    match crate::otp::verify(&state.redis, req.otp_id, &req.code).await {
        Ok(verified) => Ok(Json(VerifyOtpResponse { verified })),
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("expired") || msg.contains("not found") {
                Err((StatusCode::GONE, msg))
            } else if msg.contains("too many") {
                Err((StatusCode::TOO_MANY_REQUESTS, msg))
            } else {
                Err((StatusCode::INTERNAL_SERVER_ERROR, msg))
            }
        }
    }
}
