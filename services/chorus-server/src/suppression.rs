//! Suppression list helpers: recipient normalization and hot-path lookup.

use std::sync::Arc;
use uuid::Uuid;

use crate::app::AppState;
use crate::db::DbError;

/// Why a normalize call failed.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum NormalizeError {
    #[error("invalid E.164 phone number")]
    InvalidE164,
    #[error("unknown channel: {0}")]
    UnknownChannel(String),
}

/// Reasons a hot-path send may be rejected by the suppression layer.
#[derive(Debug, thiserror::Error)]
pub enum SuppressionRejection {
    #[error("recipient is suppressed: {reason}")]
    Suppressed { reason: String },
    #[error("invalid recipient")]
    InvalidRecipient,
    #[error("database error: {0}")]
    Db(#[from] DbError),
}

/// Normalize a recipient to its canonical form for storage and lookup.
pub fn normalize(channel: &str, recipient: &str) -> Result<String, NormalizeError> {
    match channel {
        "email" => Ok(recipient.trim().to_lowercase()),
        "sms" => {
            let r = recipient.trim();
            // E.164: leading '+', country code 1-9, total 8-15 digits.
            let re = regex::Regex::new(r"^\+[1-9]\d{1,14}$").expect("valid regex");
            if re.is_match(r) {
                Ok(r.to_string())
            } else {
                Err(NormalizeError::InvalidE164)
            }
        }
        other => Err(NormalizeError::UnknownChannel(other.to_string())),
    }
}

/// Hot-path check: returns Ok(()) if the recipient is allowed to receive a message.
pub async fn check_suppression(
    state: &Arc<AppState>,
    account_id: Uuid,
    channel: &str,
    recipient: &str,
) -> Result<(), SuppressionRejection> {
    let normalized =
        normalize(channel, recipient).map_err(|_| SuppressionRejection::InvalidRecipient)?;
    match state
        .suppression_repo()
        .is_suppressed(account_id, channel, &normalized)
        .await?
    {
        Some(reason) => Err(SuppressionRejection::Suppressed { reason }),
        None => Ok(()),
    }
}

/// Convert a [`SuppressionRejection`] into an HTTP error response body.
pub fn rejection_response(
    err: SuppressionRejection,
) -> (axum::http::StatusCode, axum::Json<serde_json::Value>) {
    match err {
        SuppressionRejection::Suppressed { reason } => (
            axum::http::StatusCode::UNPROCESSABLE_ENTITY,
            axum::Json(serde_json::json!({
                "error": {
                    "code": "recipient_suppressed",
                    "message": "Recipient is on the suppression list",
                    "reason": reason,
                }
            })),
        ),
        SuppressionRejection::InvalidRecipient => (
            axum::http::StatusCode::BAD_REQUEST,
            axum::Json(serde_json::json!({
                "error": { "code": "invalid_recipient" }
            })),
        ),
        SuppressionRejection::Db(e) => (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            axum::Json(serde_json::json!({ "error": { "message": e.to_string() } })),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn email_lowercases_and_trims() {
        assert_eq!(
            normalize("email", "  Alice@Example.COM  ").unwrap(),
            "alice@example.com"
        );
    }

    #[test]
    fn sms_passes_valid_e164() {
        assert_eq!(normalize("sms", "+66812345678").unwrap(), "+66812345678");
    }

    #[test]
    fn sms_trims_then_validates() {
        assert_eq!(normalize("sms", "  +14155552671 ").unwrap(), "+14155552671");
    }

    #[test]
    fn sms_rejects_no_plus() {
        assert_eq!(
            normalize("sms", "14155552671").unwrap_err(),
            NormalizeError::InvalidE164
        );
    }

    #[test]
    fn sms_rejects_leading_zero_country_code() {
        assert_eq!(
            normalize("sms", "+0123456789").unwrap_err(),
            NormalizeError::InvalidE164
        );
    }

    #[test]
    fn sms_rejects_letters() {
        assert_eq!(
            normalize("sms", "+1abc4155552671").unwrap_err(),
            NormalizeError::InvalidE164
        );
    }

    #[test]
    fn sms_rejects_too_short() {
        assert_eq!(
            normalize("sms", "+1").unwrap_err(),
            NormalizeError::InvalidE164
        );
    }

    #[test]
    fn unknown_channel_errors() {
        match normalize("voice", "anything") {
            Err(NormalizeError::UnknownChannel(c)) => assert_eq!(c, "voice"),
            other => panic!("unexpected: {other:?}"),
        }
    }
}
