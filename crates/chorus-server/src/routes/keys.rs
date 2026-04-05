use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use uuid::Uuid;

use crate::app::AppState;
use crate::auth::api_key::AccountContext;
use crate::db::ApiKey;

/// Request body for creating a new API key.
#[derive(Deserialize)]
pub struct CreateKeyRequest {
    /// Human-readable name for the key.
    pub name: String,
    /// `"live"` or `"test"`.
    pub environment: String,
}

/// Response returned when a new API key is created.
/// The full key is only shown once — store it securely.
#[derive(Serialize)]
pub struct CreateKeyResponse {
    pub key: String,
    pub id: Uuid,
    pub name: String,
    pub key_prefix: String,
    pub environment: String,
}

/// API key list item (key hash is never exposed).
#[derive(Serialize)]
pub struct KeyListItem {
    pub id: Uuid,
    pub name: String,
    pub key_prefix: String,
    pub environment: String,
    pub is_revoked: bool,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

impl From<ApiKey> for KeyListItem {
    fn from(k: ApiKey) -> Self {
        Self {
            id: k.id,
            name: k.name,
            key_prefix: k.key_prefix,
            environment: k.environment,
            is_revoked: k.is_revoked,
            created_at: k.created_at,
        }
    }
}

const KEY_LENGTH: usize = 32;

/// List all API keys for the authenticated account (redacted).
pub async fn list_keys(
    State(state): State<Arc<AppState>>,
    ctx: AccountContext,
) -> Result<Json<Vec<KeyListItem>>, (StatusCode, String)> {
    let keys = state
        .api_key_repo()
        .list_by_account(ctx.account_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(keys.into_iter().map(KeyListItem::from).collect()))
}

/// Create a new API key. The full key is returned only once.
pub async fn create_key(
    State(state): State<Arc<AppState>>,
    ctx: AccountContext,
    Json(req): Json<CreateKeyRequest>,
) -> Result<(StatusCode, Json<CreateKeyResponse>), (StatusCode, String)> {
    if req.environment != "live" && req.environment != "test" {
        return Err((
            StatusCode::BAD_REQUEST,
            "environment must be 'live' or 'test'".into(),
        ));
    }

    // Generate random key bytes
    let random_bytes: Vec<u8> = (0..KEY_LENGTH).map(|_| rand::random::<u8>()).collect();
    let random_hex = hex::encode(&random_bytes);

    let prefix = format!("ch_{}_", req.environment);
    let full_key = format!("{prefix}{random_hex}");
    let key_prefix = format!("{prefix}{}...", &random_hex[..8]);
    let key_hash = hex::encode(Sha256::digest(full_key.as_bytes()));

    let api_key = state
        .api_key_repo()
        .insert(ctx.account_id, &req.name, &key_hash, &key_prefix, &req.environment)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok((
        StatusCode::CREATED,
        Json(CreateKeyResponse {
            key: full_key,
            id: api_key.id,
            name: api_key.name,
            key_prefix: api_key.key_prefix,
            environment: api_key.environment,
        }),
    ))
}

/// Revoke an API key (soft delete).
pub async fn revoke_key(
    State(state): State<Arc<AppState>>,
    ctx: AccountContext,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, (StatusCode, String)> {
    state
        .api_key_repo()
        .revoke(id, ctx.account_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(StatusCode::NO_CONTENT)
}
