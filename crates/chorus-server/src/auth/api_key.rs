use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::http::StatusCode;
use sha2::{Digest, Sha256};
use std::sync::Arc;
use uuid::Uuid;

use crate::app::AppState;

/// Authenticated request context extracted from the API key.
#[derive(Debug, Clone)]
pub struct AccountContext {
    /// The account this request belongs to.
    pub account_id: Uuid,
    /// The API key used to authenticate.
    pub key_id: Uuid,
    /// `"live"` or `"test"` — determines whether messages are actually sent.
    pub environment: String,
}

impl FromRequestParts<Arc<AppState>> for AccountContext {
    type Rejection = (StatusCode, &'static str);

    async fn from_request_parts(
        parts: &mut Parts,
        state: &Arc<AppState>,
    ) -> Result<Self, Self::Rejection> {
        let header = parts
            .headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .ok_or((StatusCode::UNAUTHORIZED, "missing authorization header"))?;

        let key = header
            .strip_prefix("Bearer ")
            .ok_or((StatusCode::UNAUTHORIZED, "invalid authorization format"))?;

        if !key.starts_with("ch_live_") && !key.starts_with("ch_test_") {
            return Err((StatusCode::UNAUTHORIZED, "invalid api key format"));
        }

        let hash = hex::encode(Sha256::digest(key.as_bytes()));

        let (account, api_key) = state
            .account_repo()
            .find_by_api_key_hash(&hash)
            .await
            .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "database error"))?
            .ok_or((StatusCode::UNAUTHORIZED, "invalid api key"))?;

        if !account.is_active || api_key.is_revoked {
            return Err((StatusCode::UNAUTHORIZED, "account or key is inactive"));
        }

        if let Some(expires_at) = api_key.expires_at {
            if expires_at < chrono::Utc::now() {
                return Err((StatusCode::UNAUTHORIZED, "api key expired"));
            }
        }

        // Update last_used_at in background — fire and forget
        let repo = state.account_repo();
        let key_id = api_key.id;
        tokio::spawn(async move {
            let _ = repo.update_key_last_used(key_id).await;
        });

        Ok(AccountContext {
            account_id: account.id,
            key_id: api_key.id,
            environment: api_key.environment,
        })
    }
}
