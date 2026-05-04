//! HTTP-layer helpers for the `Idempotency-Key` header.
//!
//! See `docs/superpowers/specs/2026-05-01-idempotency-keys-design.md` for
//! the full design.

use axum::body::Bytes;
use axum::http::{HeaderMap, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use std::time::Instant;
use uuid::Uuid;

use crate::app::AppState;
use crate::db::{DbError, IdempotencyOutcome};

/// Prometheus metric name for outcome counters (labeled by `outcome`).
const METRIC_OUTCOMES: &str = "chorus_idempotency_outcomes_total";
/// Prometheus metric name for the lookup-duration histogram.
const METRIC_LOOKUP_DURATION: &str = "chorus_idempotency_lookup_duration_seconds";

fn record_outcome(outcome: &'static str) {
    metrics::counter!(METRIC_OUTCOMES, "outcome" => outcome).increment(1);
}

/// HTTP header name used to carry the idempotency key.
pub const HEADER_NAME: &str = "Idempotency-Key";

/// Maximum size of a request body that will be hashed and recorded.
/// Larger requests are rejected with 413 before idempotency runs.
pub const MAX_REQUEST_BODY_BYTES: usize = 1 << 20; // 1 MiB

/// Maximum size of a response body cached for replay.
/// Larger responses are returned to the client but **not** cached.
pub const MAX_RESPONSE_BODY_BYTES: usize = 1 << 16; // 64 KiB

/// What the route handler should do after `begin`.
pub enum IdempotencyAction {
    /// No idempotency header — proceed without recording.
    Skip,
    /// Fresh request — proceed normally and call `finalize` after.
    Proceed { token: IdempotencyToken },
    /// Return the given response immediately without executing the handler.
    Respond { status: StatusCode, body: Bytes },
}

/// Opaque token returned by `begin` and consumed by `finalize`.
pub struct IdempotencyToken {
    pub(crate) api_key_id: Uuid,
    pub(crate) key: String,
}

/// SHA-256 of the request body bytes.
pub fn sha256(body: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(body);
    hasher.finalize().into()
}

/// Returns true if `s` is a valid idempotency key:
/// non-empty, ≤255 chars, ASCII printable (graphic + space).
pub fn is_valid_key(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 255
        && s.chars().all(|c| c.is_ascii_graphic() || c == ' ')
}

/// Build the JSON body for an idempotency error response.
pub(crate) fn error_body(code: &str, message: &str) -> Bytes {
    let json = serde_json::json!({ "error": { "code": code, "message": message } });
    Bytes::from(serde_json::to_vec(&json).unwrap())
}

/// 422 response used when the same key is reused with a different body.
pub fn hash_mismatch_response() -> (StatusCode, Bytes) {
    (
        StatusCode::UNPROCESSABLE_ENTITY,
        error_body(
            "idempotency_key_reused",
            "This Idempotency-Key was used with a different request body",
        ),
    )
}

/// 400 response used when the header value fails validation.
pub fn invalid_key_response() -> (StatusCode, Bytes) {
    (
        StatusCode::BAD_REQUEST,
        error_body(
            "invalid_idempotency_key",
            "Idempotency-Key must be 1-255 ASCII printable characters",
        ),
    )
}

/// 503 response used when an in-flight retry waits past statement_timeout.
pub fn concurrent_request_response() -> (StatusCode, Bytes) {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        error_body(
            "concurrent_request",
            "Another request with this Idempotency-Key is in progress",
        ),
    )
}

/// 500 response used for unexpected DB errors.
pub fn internal_error_response(msg: &str) -> (StatusCode, Bytes) {
    (StatusCode::INTERNAL_SERVER_ERROR, error_body("internal", msg))
}

/// Inspect the `Idempotency-Key` header and decide what the route should do.
pub async fn begin(
    state: &Arc<AppState>,
    api_key_id: Uuid,
    headers: &HeaderMap,
    method: &Method,
    path: &str,
    body_bytes: &[u8],
) -> IdempotencyAction {
    let Some(raw) = headers.get(HEADER_NAME) else {
        record_outcome("skip");
        return IdempotencyAction::Skip;
    };
    let Ok(key) = raw.to_str() else {
        record_outcome("invalid_key");
        let (status, body) = invalid_key_response();
        return IdempotencyAction::Respond { status, body };
    };
    if !is_valid_key(key) {
        record_outcome("invalid_key");
        let (status, body) = invalid_key_response();
        return IdempotencyAction::Respond { status, body };
    }

    let hash = sha256(body_bytes);
    let start = Instant::now();
    let result = state
        .idempotency_repo()
        .begin(api_key_id, key, &hash, method.as_str(), path)
        .await;
    metrics::histogram!(METRIC_LOOKUP_DURATION).record(start.elapsed().as_secs_f64());

    match result {
        Ok(IdempotencyOutcome::Fresh) => {
            record_outcome("fresh");
            IdempotencyAction::Proceed {
                token: IdempotencyToken {
                    api_key_id,
                    key: key.to_string(),
                },
            }
        }
        Ok(IdempotencyOutcome::Replay { status, body }) => {
            record_outcome("replay");
            IdempotencyAction::Respond {
                status: StatusCode::from_u16(status).unwrap_or(StatusCode::OK),
                body: Bytes::from(body),
            }
        }
        Ok(IdempotencyOutcome::HashMismatch) => {
            record_outcome("hash_mismatch");
            let (status, body) = hash_mismatch_response();
            IdempotencyAction::Respond { status, body }
        }
        Err(DbError::Timeout) => {
            record_outcome("timeout");
            let (status, body) = concurrent_request_response();
            IdempotencyAction::Respond { status, body }
        }
        Err(e) => {
            record_outcome("error");
            tracing::error!(error = %e, "idempotency begin failed");
            let (status, body) = internal_error_response("idempotency lookup failed");
            IdempotencyAction::Respond { status, body }
        }
    }
}

/// Cache the response (if a token is held) and turn it into an axum [`Response`].
///
/// Used by route handlers to combine `finalize` and response construction
/// at every exit path.
pub async fn finalize_and_respond(
    state: &Arc<AppState>,
    token: Option<IdempotencyToken>,
    status: StatusCode,
    body: Bytes,
) -> Response {
    if let Some(t) = token {
        finalize(state, t, status, &body).await;
    }
    (status, body).into_response()
}

/// Build a 400 Bad Request response carrying a JSON `error` envelope.
pub fn bad_request(message: impl Into<String>) -> (StatusCode, Bytes) {
    (
        StatusCode::BAD_REQUEST,
        error_body("bad_request", &message.into()),
    )
}

/// Build a 500 Internal Server Error response carrying a JSON `error` envelope.
pub fn internal_error(message: impl Into<String>) -> (StatusCode, Bytes) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        error_body("internal", &message.into()),
    )
}

/// Persist the response so future retries with the same key replay it.
///
/// Logs but does not propagate errors — by the time `finalize` runs, the
/// downstream side effect (message insert + enqueue) has already happened,
/// and the client's response should be returned regardless.
pub async fn finalize(
    state: &Arc<AppState>,
    token: IdempotencyToken,
    status: StatusCode,
    body: &[u8],
) {
    if body.len() > MAX_RESPONSE_BODY_BYTES {
        tracing::warn!(
            size = body.len(),
            limit = MAX_RESPONSE_BODY_BYTES,
            "idempotency: response too large to cache; replay will be treated as fresh"
        );
        return;
    }
    if let Err(e) = state
        .idempotency_repo()
        .complete(token.api_key_id, &token.key, status.as_u16(), body)
        .await
    {
        tracing::warn!(error = %e, "idempotency finalize failed");
    }
}

/// Periodically delete expired idempotency rows.
///
/// Runs every 5 minutes; deletes up to 10 000 expired rows per tick to bound
/// lock contention. Logs at info on success, warn on error.
pub async fn cleanup_loop(state: Arc<AppState>) {
    const TICK: std::time::Duration = std::time::Duration::from_secs(300);
    const BATCH: i64 = 10_000;

    let mut interval = tokio::time::interval(TICK);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        interval.tick().await;
        match state.idempotency_repo().delete_expired(BATCH).await {
            Ok(n) if n > 0 => tracing::info!(deleted = n, "idempotency cleanup"),
            Ok(_) => tracing::debug!("idempotency cleanup: nothing to delete"),
            Err(e) => tracing::warn!(error = %e, "idempotency cleanup failed"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_known_empty_value() {
        let h = sha256(b"");
        assert_eq!(
            hex::encode(h),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn sha256_distinguishes_whitespace() {
        let a = sha256(b"{\"to\":\"+66\"}");
        let b = sha256(b"{ \"to\":\"+66\" }");
        assert_ne!(a, b, "whitespace must affect hash");
    }

    #[test]
    fn is_valid_key_accepts_typical_keys() {
        assert!(is_valid_key("abc-123_xyz"));
        assert!(is_valid_key("550e8400-e29b-41d4-a716-446655440000"));
        assert!(is_valid_key("key with space"));
        assert!(is_valid_key(&"a".repeat(255)));
    }

    #[test]
    fn is_valid_key_rejects_bad_inputs() {
        assert!(!is_valid_key(""));
        assert!(!is_valid_key(&"a".repeat(256)));
        assert!(!is_valid_key("key\nwith\nnewline"));
        assert!(!is_valid_key("key\twith\ttab"));
        assert!(!is_valid_key("คีย์")); // non-ASCII
    }
}
