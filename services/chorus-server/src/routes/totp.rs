use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::app::AppState;
use crate::auth::api_key::AccountContext;
use crate::db::{NewTotpUser, TotpUser};
use crate::idempotency::{self, IdempotencyAction, IdempotencyToken};
use crate::totp::{self, RateLimitKind, TotpError};

use axum::routing::{get, post};
use axum::Router;

/// Build the TOTP sub-router.
pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/enroll", post(enroll_totp))
        .route("/activate", post(activate_totp))
        .route("/verify", post(verify_totp))
        .route("/{user_id}", get(get_totp_status).delete(disenroll_totp))
        .route("/backup-codes/regenerate", post(regenerate_backup_codes))
        .route("/{user_id}/qr", get(get_totp_qr))
}

const ENROLL_PATH: &str = "/v1/totp/enroll";

#[derive(Deserialize)]
pub struct EnrollRequest {
    pub user_id: String,
    pub issuer: Option<String>,
    pub label: Option<String>,
}

#[derive(Serialize)]
pub struct EnrollResponse {
    #[serde(flatten)]
    pub user: TotpUserPublic,
    pub otpauth_uri: String,
    pub qr_code_png: String,
    pub backup_codes: Vec<String>,
    pub cost_micro: i64,
}

/// Public view of TotpUser — strips the secret.
#[derive(Serialize)]
pub struct TotpUserPublic {
    pub user_id: String,
    pub status: String,
    pub issuer: Option<String>,
    pub label: Option<String>,
    pub last_verified_at: Option<chrono::DateTime<chrono::Utc>>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub activated_at: Option<chrono::DateTime<chrono::Utc>>,
    pub unused_backup_codes_count: i64,
}

impl TotpUserPublic {
    fn from(u: &TotpUser, unused: i64) -> Self {
        Self {
            user_id: u.user_id.clone(),
            status: u.status.clone(),
            issuer: u.issuer.clone(),
            label: u.label.clone(),
            last_verified_at: u.last_verified_at,
            created_at: u.created_at,
            activated_at: u.activated_at,
            unused_backup_codes_count: unused,
        }
    }
}

#[derive(Deserialize)]
pub struct ActivateRequest {
    pub user_id: String,
    pub code: String,
}

#[derive(Deserialize)]
pub struct VerifyRequest {
    pub user_id: String,
    pub code: String,
}

#[derive(Serialize)]
pub struct VerifyResponse {
    pub user_id: String,
    pub status: String,
    pub verified: bool,
    pub method: &'static str, // "totp" | "backup_code"
    pub low_backup_codes: bool,
    pub cost_micro: i64,
}

/// POST /v1/totp/enroll
pub async fn enroll_totp(
    State(state): State<Arc<AppState>>,
    ctx: AccountContext,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let start = std::time::Instant::now();
    let response = enroll_totp_inner(state, ctx, headers, body).await;
    metrics::histogram!(totp::metrics_keys::ENROLL_DURATION).record(start.elapsed().as_secs_f64());
    response
}

async fn enroll_totp_inner(
    state: Arc<AppState>,
    ctx: AccountContext,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let token = match idempotency::begin(
        &state,
        ctx.key_id,
        &headers,
        &Method::POST,
        ENROLL_PATH,
        &body,
    )
    .await
    {
        IdempotencyAction::Skip => None,
        IdempotencyAction::Proceed { token } => Some(token),
        IdempotencyAction::Respond { status, body: b } => {
            return idempotency::finalize_and_respond(&state, None, status, b).await
        }
    };

    let req: EnrollRequest = match serde_json::from_slice(&body) {
        Ok(r) => r,
        Err(e) => {
            let (s, b) = idempotency::bad_request(e.to_string());
            return idempotency::finalize_and_respond(&state, token, s, b).await;
        }
    };

    if !totp::is_valid_user_id(&req.user_id) {
        let (s, b) = error_json(
            StatusCode::BAD_REQUEST,
            "invalid_user_id",
            "user_id must be 1-255 ASCII printable characters",
        );
        return idempotency::finalize_and_respond(&state, token, s, b).await;
    }
    let user_id = req.user_id.trim().to_string();

    let user_id_hash = totp::hash_user_id(&user_id);
    if let Err(e) = totp::check_rate_limits(
        &state.redis,
        ctx.account_id,
        &user_id_hash,
        RateLimitKind::Enroll,
    )
    .await
    {
        return route_totp_error(&state, token, e).await;
    }

    // Reject if already pending or active.
    match state.totp_repo().find(ctx.account_id, &user_id).await {
        Ok(Some(existing)) if existing.status != "disabled" => {
            metrics::counter!(totp::metrics_keys::ENROLLMENTS_TOTAL, "outcome" => "already_enrolled").increment(1);
            let (s, b) = error_json(
                StatusCode::CONFLICT,
                "already_enrolled",
                "user is already enrolled",
            );
            return idempotency::finalize_and_respond(&state, token, s, b).await;
        }
        Ok(Some(_)) => {
            // disabled — re-enroll path: clear old row first
            if let Err(e) = clear_disabled(&state, ctx.account_id, &user_id).await {
                let (s, b) = idempotency::internal_error(e.to_string());
                return idempotency::finalize_and_respond(&state, token, s, b).await;
            }
        }
        Ok(None) => {}
        Err(e) => {
            let (s, b) = idempotency::internal_error(e.to_string());
            return idempotency::finalize_and_respond(&state, token, s, b).await;
        }
    }

    let secret = totp::generate_secret();
    let encrypted = match state.encryptor().encrypt(&secret) {
        Ok(b) => b,
        Err(e) => {
            let (s, b) = idempotency::internal_error(e.to_string());
            return idempotency::finalize_and_respond(&state, token, s, b).await;
        }
    };
    let backup_plaintext = totp::generate_backup_codes();
    let backup_hashes: Vec<Vec<u8>> = backup_plaintext
        .iter()
        .map(|c| totp::hash_backup_code(c))
        .collect();

    let issuer = req.issuer.clone();
    let label = req.label.clone();
    let new_user = NewTotpUser {
        account_id: ctx.account_id,
        user_id: user_id.clone(),
        encrypted_secret: encrypted,
        issuer: issuer.clone(),
        label: label.clone(),
    };
    let user = match state.totp_repo().enroll(&new_user, &backup_hashes).await {
        Ok(u) => u,
        Err(e) => {
            let (s, b) = idempotency::internal_error(e.to_string());
            return idempotency::finalize_and_respond(&state, token, s, b).await;
        }
    };

    let resolved_issuer = issuer.unwrap_or_else(|| "Chorus".to_string());
    let resolved_label = label.unwrap_or_else(|| user_id.clone());
    let secret_b32 = totp::base32_no_pad(&secret);
    let otpauth = totp::build_otpauth_uri(&resolved_issuer, &resolved_label, &secret_b32);
    let qr_png = match totp::qr_png_data_uri(&otpauth) {
        Ok(s) => s,
        Err(e) => {
            let (s, b) = idempotency::internal_error(e.to_string());
            return idempotency::finalize_and_respond(&state, token, s, b).await;
        }
    };

    let response = EnrollResponse {
        user: TotpUserPublic::from(&user, backup_plaintext.len() as i64),
        otpauth_uri: otpauth,
        qr_code_png: qr_png,
        backup_codes: backup_plaintext,
        cost_micro: 0,
    };
    let bytes = Bytes::from(serde_json::to_vec(&response).unwrap_or_default());
    metrics::counter!(totp::metrics_keys::ENROLLMENTS_TOTAL, "outcome" => "created").increment(1);
    idempotency::finalize_and_respond(&state, token, StatusCode::CREATED, bytes).await
}

async fn clear_disabled(
    state: &Arc<AppState>,
    account_id: uuid::Uuid,
    user_id: &str,
) -> Result<(), crate::db::DbError> {
    // The repo's enroll path uses INSERT with PK — we must delete the disabled row first.
    let pool = match &state.db {
        Some(p) => p,
        None => return Ok(()), // tests using mocks won't hit this branch
    };
    sqlx::query("DELETE FROM totp_users WHERE account_id=$1 AND user_id=$2")
        .bind(account_id)
        .bind(user_id)
        .execute(pool)
        .await
        .map_err(|e| crate::db::DbError::Internal(anyhow::Error::from(e)))?;
    Ok(())
}

/// POST /v1/totp/activate
pub async fn activate_totp(
    State(state): State<Arc<AppState>>,
    ctx: AccountContext,
    Json(req): Json<ActivateRequest>,
) -> Response {
    if !totp::is_valid_user_id(&req.user_id) {
        return error_response(
            StatusCode::BAD_REQUEST,
            "invalid_user_id",
            "user_id must be 1-255 ASCII printable characters",
        );
    }
    let user_id = req.user_id.trim();
    let user_id_hash = totp::hash_user_id(user_id);

    if let Err(e) = totp::check_rate_limits(
        &state.redis,
        ctx.account_id,
        &user_id_hash,
        RateLimitKind::Activate,
    )
    .await
    {
        return route_totp_error_plain(e);
    }

    let user = match state.totp_repo().find(ctx.account_id, user_id).await {
        Ok(Some(u)) => u,
        Ok(None) => return error_response(StatusCode::NOT_FOUND, "not_found", "user not enrolled"),
        Err(e) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal",
                &e.to_string(),
            )
        }
    };
    if user.status != "pending" {
        return error_response(
            StatusCode::GONE,
            "not_pending",
            "user is not pending activation",
        );
    }

    let secret = match state.encryptor().decrypt(&user.secret) {
        Ok(s) => s,
        Err(e) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal",
                &e.to_string(),
            )
        }
    };
    let now = unix_now();
    if !totp::verify_totp_with_window(&secret, now, &req.code) {
        return error_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            "incorrect_code",
            "code did not match",
        );
    }

    if let Err(e) = state.totp_repo().activate(ctx.account_id, user_id).await {
        return error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal",
            &e.to_string(),
        );
    }
    let user = match state.totp_repo().find(ctx.account_id, user_id).await {
        Ok(Some(u)) => u,
        _ => user,
    };
    let unused = state
        .totp_repo()
        .unused_backup_codes_count(ctx.account_id, user_id)
        .await
        .unwrap_or(0);
    (StatusCode::OK, Json(TotpUserPublic::from(&user, unused))).into_response()
}

/// POST /v1/totp/verify
pub async fn verify_totp(
    State(state): State<Arc<AppState>>,
    ctx: AccountContext,
    Json(req): Json<VerifyRequest>,
) -> Response {
    let start = std::time::Instant::now();
    let response = verify_totp_inner(state, ctx, req).await;
    metrics::histogram!(totp::metrics_keys::VERIFY_DURATION).record(start.elapsed().as_secs_f64());
    response
}

async fn verify_totp_inner(
    state: Arc<AppState>,
    ctx: AccountContext,
    req: VerifyRequest,
) -> Response {
    if !totp::is_valid_user_id(&req.user_id) {
        return error_response(
            StatusCode::BAD_REQUEST,
            "invalid_user_id",
            "user_id must be 1-255 ASCII printable characters",
        );
    }
    let user_id = req.user_id.trim();
    let user_id_hash = totp::hash_user_id(user_id);

    if let Err(e) = totp::check_rate_limits(
        &state.redis,
        ctx.account_id,
        &user_id_hash,
        RateLimitKind::Verify,
    )
    .await
    {
        return route_totp_error_plain(e);
    }

    let user = match state.totp_repo().find(ctx.account_id, user_id).await {
        Ok(Some(u)) => u,
        Ok(None) => return error_response(StatusCode::NOT_FOUND, "not_found", "user not enrolled"),
        Err(e) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal",
                &e.to_string(),
            )
        }
    };
    if user.status != "active" {
        return error_response(StatusCode::GONE, "not_active", "user is not active");
    }

    if totp::is_backup_code_format(&req.code) {
        let hash = totp::hash_backup_code(&req.code);
        let ok = match state
            .totp_repo()
            .consume_backup_code(ctx.account_id, user_id, &hash)
            .await
        {
            Ok(v) => v,
            Err(e) => {
                return error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal",
                    &e.to_string(),
                )
            }
        };
        if !ok {
            metrics::counter!(
                totp::metrics_keys::VERIFIES_TOTAL,
                "outcome" => "wrong_code",
                "method" => "backup_code"
            )
            .increment(1);
            return error_response(
                StatusCode::UNPROCESSABLE_ENTITY,
                "incorrect_code",
                "code did not match",
            );
        }
        let _ = state
            .totp_repo()
            .touch_last_verified(ctx.account_id, user_id)
            .await;
        let unused = state
            .totp_repo()
            .unused_backup_codes_count(ctx.account_id, user_id)
            .await
            .unwrap_or(0);
        metrics::counter!(
            totp::metrics_keys::VERIFIES_TOTAL,
            "outcome" => "approved",
            "method" => "backup_code"
        )
        .increment(1);
        metrics::gauge!(totp::metrics_keys::BACKUP_REMAINING).set(unused as f64);
        return Json(VerifyResponse {
            user_id: user.user_id.clone(),
            status: "active".into(),
            verified: true,
            method: "backup_code",
            low_backup_codes: unused < totp::LOW_BACKUP_THRESHOLD,
            cost_micro: 0,
        })
        .into_response();
    }

    let secret = match state.encryptor().decrypt(&user.secret) {
        Ok(s) => s,
        Err(e) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal",
                &e.to_string(),
            )
        }
    };
    let now = unix_now();
    if !totp::verify_totp_with_window(&secret, now, &req.code) {
        metrics::counter!(
            totp::metrics_keys::VERIFIES_TOTAL,
            "outcome" => "wrong_code",
            "method" => "totp"
        )
        .increment(1);
        return error_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            "incorrect_code",
            "code did not match",
        );
    }
    let _ = state
        .totp_repo()
        .touch_last_verified(ctx.account_id, user_id)
        .await;
    let unused = state
        .totp_repo()
        .unused_backup_codes_count(ctx.account_id, user_id)
        .await
        .unwrap_or(0);
    metrics::counter!(
        totp::metrics_keys::VERIFIES_TOTAL,
        "outcome" => "approved",
        "method" => "totp"
    )
    .increment(1);
    metrics::gauge!(totp::metrics_keys::BACKUP_REMAINING).set(unused as f64);
    Json(VerifyResponse {
        user_id: user.user_id.clone(),
        status: "active".into(),
        verified: true,
        method: "totp",
        low_backup_codes: unused < totp::LOW_BACKUP_THRESHOLD,
        cost_micro: 0,
    })
    .into_response()
}

/// DELETE /v1/totp/{user_id}
pub async fn disenroll_totp(
    State(state): State<Arc<AppState>>,
    ctx: AccountContext,
    Path(user_id): Path<String>,
) -> Response {
    if !totp::is_valid_user_id(&user_id) {
        return error_response(
            StatusCode::BAD_REQUEST,
            "invalid_user_id",
            "user_id must be 1-255 ASCII printable characters",
        );
    }
    match state
        .totp_repo()
        .disenroll(ctx.account_id, user_id.trim())
        .await
    {
        Ok(true) => {
            let body = serde_json::json!({"user_id": user_id, "status": "disabled"});
            (StatusCode::OK, Json(body)).into_response()
        }
        Ok(false) => error_response(
            StatusCode::GONE,
            "not_found_or_already_disabled",
            "no active TOTP enrollment found",
        ),
        Err(e) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal",
            &e.to_string(),
        ),
    }
}

/// GET /v1/totp/{user_id}
pub async fn get_totp_status(
    State(state): State<Arc<AppState>>,
    ctx: AccountContext,
    Path(user_id): Path<String>,
) -> Response {
    if !totp::is_valid_user_id(&user_id) {
        return error_response(
            StatusCode::BAD_REQUEST,
            "invalid_user_id",
            "user_id must be 1-255 ASCII printable characters",
        );
    }
    let user_id = user_id.trim();
    let user = match state.totp_repo().find(ctx.account_id, user_id).await {
        Ok(Some(u)) => u,
        Ok(None) => return error_response(StatusCode::NOT_FOUND, "not_found", "user not enrolled"),
        Err(e) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal",
                &e.to_string(),
            )
        }
    };
    let unused = state
        .totp_repo()
        .unused_backup_codes_count(ctx.account_id, user_id)
        .await
        .unwrap_or(0);
    (StatusCode::OK, Json(TotpUserPublic::from(&user, unused))).into_response()
}

const REGEN_PATH: &str = "/v1/totp/backup-codes/regenerate";

#[derive(Deserialize)]
pub struct RegenerateRequest {
    pub user_id: String,
}

#[derive(Serialize)]
pub struct RegenerateResponse {
    pub user_id: String,
    pub backup_codes: Vec<String>,
    pub cost_micro: i64,
}

/// POST /v1/totp/backup-codes/regenerate
pub async fn regenerate_backup_codes(
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
        REGEN_PATH,
        &body,
    )
    .await
    {
        IdempotencyAction::Skip => None,
        IdempotencyAction::Proceed { token } => Some(token),
        IdempotencyAction::Respond { status, body: b } => {
            return idempotency::finalize_and_respond(&state, None, status, b).await
        }
    };

    let req: RegenerateRequest = match serde_json::from_slice(&body) {
        Ok(r) => r,
        Err(e) => {
            let (s, b) = idempotency::bad_request(e.to_string());
            return idempotency::finalize_and_respond(&state, token, s, b).await;
        }
    };
    if !totp::is_valid_user_id(&req.user_id) {
        let (s, b) = error_json(
            StatusCode::BAD_REQUEST,
            "invalid_user_id",
            "user_id must be 1-255 ASCII printable characters",
        );
        return idempotency::finalize_and_respond(&state, token, s, b).await;
    }
    let user_id = req.user_id.trim().to_string();

    let user_id_hash = totp::hash_user_id(&user_id);
    if let Err(e) = totp::check_rate_limits(
        &state.redis,
        ctx.account_id,
        &user_id_hash,
        RateLimitKind::Enroll,
    )
    .await
    {
        return route_totp_error(&state, token, e).await;
    }

    let user = match state.totp_repo().find(ctx.account_id, &user_id).await {
        Ok(Some(u)) => u,
        Ok(None) => {
            let (s, b) = error_json(StatusCode::NOT_FOUND, "not_found", "user not enrolled");
            return idempotency::finalize_and_respond(&state, token, s, b).await;
        }
        Err(e) => {
            let (s, b) = idempotency::internal_error(e.to_string());
            return idempotency::finalize_and_respond(&state, token, s, b).await;
        }
    };
    if user.status != "active" {
        let (s, b) = error_json(StatusCode::GONE, "not_active", "user is not active");
        return idempotency::finalize_and_respond(&state, token, s, b).await;
    }

    let new_plaintext = totp::generate_backup_codes();
    let new_hashes: Vec<Vec<u8>> = new_plaintext
        .iter()
        .map(|c| totp::hash_backup_code(c))
        .collect();
    if let Err(e) = state
        .totp_repo()
        .replace_backup_codes(ctx.account_id, &user_id, &new_hashes)
        .await
    {
        let (s, b) = idempotency::internal_error(e.to_string());
        return idempotency::finalize_and_respond(&state, token, s, b).await;
    }

    let response = RegenerateResponse {
        user_id: user_id.clone(),
        backup_codes: new_plaintext,
        cost_micro: 0,
    };
    let bytes = Bytes::from(serde_json::to_vec(&response).unwrap_or_default());
    idempotency::finalize_and_respond(&state, token, StatusCode::OK, bytes).await
}

/// GET /v1/totp/{user_id}/qr — raw image/png response
pub async fn get_totp_qr(
    State(state): State<Arc<AppState>>,
    ctx: AccountContext,
    Path(user_id): Path<String>,
) -> Response {
    if !totp::is_valid_user_id(&user_id) {
        return error_response(
            StatusCode::BAD_REQUEST,
            "invalid_user_id",
            "user_id must be 1-255 ASCII printable characters",
        );
    }
    let user_id = user_id.trim();
    let user = match state.totp_repo().find(ctx.account_id, user_id).await {
        Ok(Some(u)) => u,
        Ok(None) => return error_response(StatusCode::NOT_FOUND, "not_found", "user not enrolled"),
        Err(e) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal",
                &e.to_string(),
            )
        }
    };
    if user.status == "disabled" {
        return error_response(StatusCode::NOT_FOUND, "not_found", "user not enrolled");
    }
    let secret = match state.encryptor().decrypt(&user.secret) {
        Ok(s) => s,
        Err(e) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal",
                &e.to_string(),
            )
        }
    };
    let issuer = user.issuer.clone().unwrap_or_else(|| "Chorus".to_string());
    let label = user.label.clone().unwrap_or_else(|| user.user_id.clone());
    let otpauth = totp::build_otpauth_uri(&issuer, &label, &totp::base32_no_pad(&secret));
    let png_data_uri = match totp::qr_png_data_uri(&otpauth) {
        Ok(s) => s,
        Err(e) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal",
                &e.to_string(),
            )
        }
    };
    use base64::Engine;
    let b64 = png_data_uri
        .strip_prefix("data:image/png;base64,")
        .unwrap_or("");
    let png_bytes = base64::engine::general_purpose::STANDARD
        .decode(b64)
        .unwrap_or_default();
    (
        StatusCode::OK,
        [(axum::http::header::CONTENT_TYPE, "image/png")],
        png_bytes,
    )
        .into_response()
}

// ---- internal helpers ----

fn unix_now() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn error_response(status: StatusCode, code: &str, message: &str) -> Response {
    let body = serde_json::json!({ "error": { "code": code, "message": message } });
    (status, Json(body)).into_response()
}

fn error_json(status: StatusCode, code: &str, message: &str) -> (StatusCode, Bytes) {
    let body = serde_json::json!({ "error": { "code": code, "message": message } });
    (
        status,
        Bytes::from(serde_json::to_vec(&body).unwrap_or_default()),
    )
}

async fn route_totp_error(
    state: &Arc<AppState>,
    token: Option<IdempotencyToken>,
    err: TotpError,
) -> Response {
    match err {
        TotpError::RateLimitedUser { retry_after_sec }
        | TotpError::RateLimitedAccount { retry_after_sec } => {
            let (s, b) = error_json(
                StatusCode::TOO_MANY_REQUESTS,
                "rate_limited",
                "TOTP rate limit exceeded",
            );
            if let Some(t) = token {
                idempotency::finalize(state, t, s, &b).await;
            }
            let mut resp = (s, b).into_response();
            if let Ok(v) = HeaderValue::from_str(&retry_after_sec.to_string()) {
                resp.headers_mut()
                    .insert(axum::http::header::RETRY_AFTER, v);
            }
            resp
        }
        TotpError::Db(e) => {
            let (s, b) = idempotency::internal_error(e.to_string());
            idempotency::finalize_and_respond(state, token, s, b).await
        }
        TotpError::Internal(e) => {
            let (s, b) = idempotency::internal_error(e.to_string());
            idempotency::finalize_and_respond(state, token, s, b).await
        }
        // Handler-specific variants not produced by check_rate_limits.
        TotpError::InvalidUserId
        | TotpError::AlreadyEnrolled
        | TotpError::NotFound
        | TotpError::NotPending
        | TotpError::NotActive
        | TotpError::IncorrectCode => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal",
            "unexpected TotpError variant",
        ),
    }
}

fn route_totp_error_plain(err: TotpError) -> Response {
    match err {
        TotpError::RateLimitedUser { retry_after_sec }
        | TotpError::RateLimitedAccount { retry_after_sec } => {
            let mut resp = error_response(
                StatusCode::TOO_MANY_REQUESTS,
                "rate_limited",
                "TOTP rate limit exceeded",
            );
            if let Ok(v) = HeaderValue::from_str(&retry_after_sec.to_string()) {
                resp.headers_mut()
                    .insert(axum::http::header::RETRY_AFTER, v);
            }
            resp
        }
        TotpError::Db(e) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal",
            &e.to_string(),
        ),
        TotpError::Internal(e) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal",
            &e.to_string(),
        ),
        _ => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal",
            "unexpected TotpError variant",
        ),
    }
}
