use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::http::StatusCode;
use sha2::{Digest, Sha256};
use std::sync::Arc;
use uuid::Uuid;

use crate::app::AppState;

/// Authenticated admin context extracted from an admin API key.
#[derive(Debug, Clone)]
pub struct AdminContext {
    /// The admin key ID used to authenticate.
    pub key_id: Uuid,
}

impl FromRequestParts<Arc<AppState>> for AdminContext {
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

        if !key.starts_with("ch_admin_") {
            return Err((StatusCode::UNAUTHORIZED, "invalid admin key format"));
        }

        let hash = hex::encode(Sha256::digest(key.as_bytes()));

        let admin_key = state
            .admin_key_repo()
            .find_by_hash(&hash)
            .await
            .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "database error"))?
            .ok_or((StatusCode::UNAUTHORIZED, "invalid admin key"))?;

        if admin_key.is_revoked {
            return Err((StatusCode::UNAUTHORIZED, "admin key is revoked"));
        }

        Ok(AdminContext {
            key_id: admin_key.id,
        })
    }
}
