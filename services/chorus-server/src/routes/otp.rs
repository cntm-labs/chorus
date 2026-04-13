use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use crate::app::AppState;
use crate::auth::api_key::AccountContext;

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
pub async fn send_otp(
    State(state): State<Arc<AppState>>,
    _ctx: AccountContext,
    Json(req): Json<SendOtpRequest>,
) -> Result<(StatusCode, Json<SendOtpResponse>), (StatusCode, String)> {
    let code = crate::otp::generate_code();

    let otp_id = crate::otp::store(&state.redis, &req.to, &code)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let app_name = req.app_name.as_deref().unwrap_or("Chorus");
    let body = format!("Your {app_name} verification code is: {code}");

    // Queue the OTP message for delivery
    let new_msg = crate::db::NewMessage {
        account_id: _ctx.account_id,
        api_key_id: _ctx.key_id,
        channel: if req.to.contains('@') { "email" } else { "sms" }.into(),
        sender: None,
        recipient: req.to,
        subject: Some(format!("{app_name} verification code")),
        body,
        environment: _ctx.environment.clone(),
    };

    let message = state
        .message_repo()
        .insert(&new_msg)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let job = crate::queue::SendJob {
        message_id: message.id,
        account_id: message.account_id,
        channel: message.channel.clone(),
        environment: _ctx.environment,
        attempt: 0,
    };
    crate::queue::enqueue::enqueue_job(&state, &job)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok((StatusCode::CREATED, Json(SendOtpResponse { otp_id })))
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
