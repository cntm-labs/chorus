use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use crate::app::AppState;
use crate::auth::api_key::AccountContext;
use crate::db::{NewVerification, Pagination, Verification};
use crate::idempotency::{self, IdempotencyAction, IdempotencyToken};
use crate::queue::SendJob;
use crate::verification::{
    self, ChannelChoice, CheckCodeOutcome, RoutingError, MAX_CHECK_ATTEMPTS,
};

const CREATE_PATH: &str = "/v1/verifications";

#[derive(Deserialize)]
pub struct CreateVerificationRequest {
    pub phone: Option<String>,
    pub email: Option<String>,
    pub channels: Option<Vec<String>>,
    pub app_name: Option<String>,
}

#[derive(Deserialize)]
pub struct CheckRequest {
    pub code: String,
}

#[derive(Serialize)]
pub struct CheckResponse {
    #[serde(flatten)]
    pub verification: Verification,
    /// Only set when the result is a wrong-code miss.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attempts_remaining: Option<i32>,
}

#[derive(Deserialize)]
pub struct ListParams {
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

#[derive(Serialize)]
pub struct ListResponse {
    pub data: Vec<Verification>,
    pub limit: i64,
    pub offset: i64,
}

const DEFAULT_LIMIT: i64 = 20;
const MAX_LIMIT: i64 = 100;

/// POST /v1/verifications — create + smart-routed send.
pub async fn create_verification(
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
        CREATE_PATH,
        &body,
    )
    .await
    {
        IdempotencyAction::Skip => None,
        IdempotencyAction::Proceed { token } => Some(token),
        IdempotencyAction::Respond { status, body: b } => {
            return idempotency::finalize_and_respond(&state, None, status, b).await;
        }
    };

    let req: CreateVerificationRequest = match serde_json::from_slice(&body) {
        Ok(r) => r,
        Err(e) => {
            let (status, body) = idempotency::bad_request(e.to_string());
            return idempotency::finalize_and_respond(&state, token, status, body).await;
        }
    };

    // Rate limits — keyed on the eligible recipient. We pre-pick the rate-limit
    // recipient as the *first* non-empty of email/phone (for hash stability).
    let rl_recipient = req
        .email
        .as_deref()
        .or(req.phone.as_deref())
        .unwrap_or("");
    if rl_recipient.is_empty() {
        let (status, body) = error_json(StatusCode::BAD_REQUEST, "no_recipient", "phone or email required");
        return idempotency::finalize_and_respond(&state, token, status, body).await;
    }
    let rl_hash = verification::hash_recipient(rl_recipient);
    if let Err(e) =
        verification::check_rate_limits(&state.redis, ctx.account_id, &rl_hash).await
    {
        return route_routing_error(&state, token, e).await;
    }

    // Smart routing.
    let channels = req.channels.unwrap_or_default();
    let choice = match verification::select_channel(
        &state,
        ctx.account_id,
        req.phone.as_deref(),
        req.email.as_deref(),
        &channels,
    )
    .await
    {
        Ok(c) => c,
        Err(e) => return route_routing_error(&state, token, e).await,
    };

    // Insert + Valkey + enqueue.
    let code = verification::generate_code();
    let new_v = NewVerification {
        account_id: ctx.account_id,
        api_key_id: ctx.key_id,
        channel: choice.channel().to_string(),
        recipient: choice.recipient().to_string(),
        environment: ctx.environment.clone(),
        app_name: req.app_name.clone(),
        initial_cost_micro: choice.cost_micro(),
    };
    let v = match state.verification_repo().insert(&new_v).await {
        Ok(row) => row,
        Err(e) => {
            let (status, body) = idempotency::internal_error(e.to_string());
            return idempotency::finalize_and_respond(&state, token, status, body).await;
        }
    };

    if let Err(e) =
        verification::store_code(&state.redis, v.id, choice.recipient(), &code).await
    {
        let (status, body) = idempotency::internal_error(e.to_string());
        return idempotency::finalize_and_respond(&state, token, status, body).await;
    }

    if let Err(e) = enqueue_verification_send(&state, &ctx, &v, &choice, &code).await {
        let (status, body) = idempotency::internal_error(e.to_string());
        return idempotency::finalize_and_respond(&state, token, status, body).await;
    }

    let bytes = Bytes::from(serde_json::to_vec(&v).unwrap_or_default());
    idempotency::finalize_and_respond(&state, token, StatusCode::CREATED, bytes).await
}

/// POST /v1/verifications/{id}/check
pub async fn check_verification(
    State(state): State<Arc<AppState>>,
    ctx: AccountContext,
    Path(id): Path<Uuid>,
    Json(req): Json<CheckRequest>,
) -> Response {
    let repo = state.verification_repo();
    let v = match repo.find_by_id(id, ctx.account_id).await {
        Ok(Some(v)) => v,
        Ok(None) => return error_response(StatusCode::NOT_FOUND, "not_found", "verification not found"),
        Err(e) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, "internal", &e.to_string()),
    };
    if v.status != "pending" {
        return error_response(StatusCode::GONE, &v.status, "verification is not pending");
    }

    let new_attempts = match repo.increment_check_attempts(id, ctx.account_id).await {
        Ok(n) => n,
        Err(crate::db::DbError::NotFound) => {
            return error_response(StatusCode::GONE, "expired", "verification is no longer pending");
        }
        Err(e) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, "internal", &e.to_string()),
    };

    if new_attempts > MAX_CHECK_ATTEMPTS {
        let _ = verification::invalidate_code(&state.redis, id).await;
        let _ = repo.mark_canceled(id, ctx.account_id).await;
        return error_response(
            StatusCode::GONE,
            "max_attempts_exceeded",
            "maximum check attempts reached",
        );
    }

    match verification::check_code(&state.redis, id, &req.code).await {
        Ok(CheckCodeOutcome::Match) => {
            if let Err(e) = repo.mark_approved(id, ctx.account_id).await {
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "internal", &e.to_string());
            }
            let approved = match repo.find_by_id(id, ctx.account_id).await {
                Ok(Some(v)) => v,
                _ => v,
            };
            (StatusCode::OK, Json(approved)).into_response()
        }
        Ok(CheckCodeOutcome::Mismatch) => {
            let remaining = MAX_CHECK_ATTEMPTS - new_attempts;
            let body = serde_json::json!({
                "error": {
                    "code": "incorrect_code",
                    "message": "the provided code is incorrect",
                    "attempts_remaining": remaining.max(0),
                }
            });
            (StatusCode::UNPROCESSABLE_ENTITY, Json(body)).into_response()
        }
        Ok(CheckCodeOutcome::Gone) => {
            error_response(StatusCode::GONE, "expired", "verification code has expired")
        }
        Err(e) => error_response(StatusCode::INTERNAL_SERVER_ERROR, "internal", &e.to_string()),
    }
}

/// POST /v1/verifications/{id}/cancel
pub async fn cancel_verification(
    State(state): State<Arc<AppState>>,
    ctx: AccountContext,
    Path(id): Path<Uuid>,
) -> Response {
    let repo = state.verification_repo();
    match repo.mark_canceled(id, ctx.account_id).await {
        Ok(true) => {
            let _ = verification::invalidate_code(&state.redis, id).await;
            let row = match repo.find_by_id(id, ctx.account_id).await {
                Ok(Some(v)) => v,
                Ok(None) => return error_response(StatusCode::NOT_FOUND, "not_found", "verification not found"),
                Err(e) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, "internal", &e.to_string()),
            };
            (StatusCode::OK, Json(row)).into_response()
        }
        Ok(false) => {
            error_response(StatusCode::GONE, "already_terminal", "verification is not pending")
        }
        Err(e) => error_response(StatusCode::INTERNAL_SERVER_ERROR, "internal", &e.to_string()),
    }
}

/// GET /v1/verifications/{id}
pub async fn get_verification(
    State(state): State<Arc<AppState>>,
    ctx: AccountContext,
    Path(id): Path<Uuid>,
) -> Response {
    match state.verification_repo().find_by_id(id, ctx.account_id).await {
        Ok(Some(v)) => (StatusCode::OK, Json(v)).into_response(),
        Ok(None) => error_response(StatusCode::NOT_FOUND, "not_found", "verification not found"),
        Err(e) => error_response(StatusCode::INTERNAL_SERVER_ERROR, "internal", &e.to_string()),
    }
}

/// GET /v1/verifications
pub async fn list_verifications(
    State(state): State<Arc<AppState>>,
    ctx: AccountContext,
    Query(params): Query<ListParams>,
) -> Response {
    let limit = params.limit.unwrap_or(DEFAULT_LIMIT).min(MAX_LIMIT);
    let offset = params.offset.unwrap_or(0);
    let pagination = Pagination { limit, offset };
    match state
        .verification_repo()
        .list_by_account(ctx.account_id, &pagination)
        .await
    {
        Ok(data) => (
            StatusCode::OK,
            Json(ListResponse { data, limit, offset }),
        )
            .into_response(),
        Err(e) => error_response(StatusCode::INTERNAL_SERVER_ERROR, "internal", &e.to_string()),
    }
}

// ---- internal helpers ----

pub(crate) async fn enqueue_verification_send(
    state: &Arc<AppState>,
    ctx: &AccountContext,
    v: &Verification,
    choice: &ChannelChoice,
    code: &str,
) -> anyhow::Result<()> {
    let app_name = v.app_name.as_deref().unwrap_or("Chorus");
    let body = format!("Your {app_name} verification code is: {code}");
    let new_msg = crate::db::NewMessage {
        account_id: ctx.account_id,
        api_key_id: ctx.key_id,
        channel: choice.channel().to_string(),
        sender: None,
        recipient: choice.recipient().to_string(),
        subject: if choice.channel() == "email" {
            Some(format!("{app_name} verification code"))
        } else {
            None
        },
        body,
        environment: ctx.environment.clone(),
    };
    let message = state.message_repo().insert(&new_msg).await?;
    let job = SendJob {
        message_id: message.id,
        account_id: message.account_id,
        channel: choice.channel().to_string(),
        environment: message.environment.clone(),
        attempt: 0,
    };
    crate::queue::enqueue::notify(state, &job).await?;
    let _ = v;
    Ok(())
}

fn error_response(status: StatusCode, code: &str, message: &str) -> Response {
    let body = serde_json::json!({ "error": { "code": code, "message": message } });
    (status, Json(body)).into_response()
}

fn error_json(status: StatusCode, code: &str, message: &str) -> (StatusCode, Bytes) {
    let body = serde_json::json!({ "error": { "code": code, "message": message } });
    (status, Bytes::from(serde_json::to_vec(&body).unwrap_or_default()))
}

async fn route_routing_error(
    state: &Arc<AppState>,
    token: Option<IdempotencyToken>,
    err: RoutingError,
) -> Response {
    match err {
        RoutingError::NoRecipient => {
            let (s, b) = error_json(StatusCode::BAD_REQUEST, "no_recipient", "phone or email required");
            idempotency::finalize_and_respond(state, token, s, b).await
        }
        RoutingError::InvalidPhone => {
            let (s, b) = error_json(StatusCode::BAD_REQUEST, "invalid_phone", "phone must be E.164");
            idempotency::finalize_and_respond(state, token, s, b).await
        }
        RoutingError::InvalidEmail => {
            let (s, b) = error_json(StatusCode::BAD_REQUEST, "invalid_email", "email format is invalid");
            idempotency::finalize_and_respond(state, token, s, b).await
        }
        RoutingError::NoEligibleChannel => {
            let (s, b) = error_json(
                StatusCode::UNPROCESSABLE_ENTITY,
                "no_eligible_channel",
                "no channel is eligible (suppressed or missing)",
            );
            idempotency::finalize_and_respond(state, token, s, b).await
        }
        RoutingError::RateLimitedRecipient { retry_after_sec }
        | RoutingError::RateLimitedAccount { retry_after_sec } => {
            let (s, b) = error_json(
                StatusCode::TOO_MANY_REQUESTS,
                "rate_limited",
                "verification rate limit exceeded",
            );
            // finalize separately so cached body remains identical on replay
            if let Some(t) = token {
                idempotency::finalize(state, t, s, &b).await;
            }
            let mut resp = (s, b).into_response();
            if let Ok(v) = HeaderValue::from_str(&retry_after_sec.to_string()) {
                resp.headers_mut().insert(axum::http::header::RETRY_AFTER, v);
            }
            resp
        }
        RoutingError::Db(e) => {
            let (s, b) = idempotency::internal_error(e.to_string());
            idempotency::finalize_and_respond(state, token, s, b).await
        }
        RoutingError::Internal(e) => {
            let (s, b) = idempotency::internal_error(e.to_string());
            idempotency::finalize_and_respond(state, token, s, b).await
        }
    }
}
