use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::Deserialize;
use std::sync::Arc;
use uuid::Uuid;

use crate::app::AppState;
use crate::auth::api_key::AccountContext;
use crate::db::{NewProviderConfig, ProviderConfig};

/// Request body for adding a provider config.
#[derive(Deserialize)]
pub struct CreateProviderConfigRequest {
    pub channel: String,
    pub provider: String,
    pub priority: i32,
    pub credentials: serde_json::Value,
}

/// Valid channel names.
const VALID_CHANNELS: &[&str] = &["sms", "email"];
/// Valid provider names.
const VALID_PROVIDERS: &[&str] = &["telnyx", "twilio", "plivo", "resend", "ses", "smtp"];

/// List all provider configs for the authenticated account.
pub async fn list_provider_configs(
    State(state): State<Arc<AppState>>,
    ctx: AccountContext,
) -> Result<Json<Vec<ProviderConfig>>, (StatusCode, String)> {
    let configs = state
        .provider_config_repo()
        .list_by_account(ctx.account_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(configs))
}

/// Add a provider config for the authenticated account.
pub async fn create_provider_config(
    State(state): State<Arc<AppState>>,
    ctx: AccountContext,
    Json(req): Json<CreateProviderConfigRequest>,
) -> Result<(StatusCode, Json<ProviderConfig>), (StatusCode, String)> {
    if !VALID_CHANNELS.contains(&req.channel.as_str()) {
        return Err((StatusCode::BAD_REQUEST, "channel must be 'sms' or 'email'".into()));
    }

    if !VALID_PROVIDERS.contains(&req.provider.as_str()) {
        return Err((StatusCode::BAD_REQUEST, format!("unknown provider: {}", req.provider)));
    }

    let config = state
        .provider_config_repo()
        .insert(&NewProviderConfig {
            account_id: ctx.account_id,
            channel: req.channel,
            provider: req.provider,
            priority: req.priority,
            credentials: req.credentials,
        })
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok((StatusCode::CREATED, Json(config)))
}

/// Delete a provider config.
pub async fn delete_provider_config(
    State(state): State<Arc<AppState>>,
    ctx: AccountContext,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, (StatusCode, String)> {
    state
        .provider_config_repo()
        .delete(id, ctx.account_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(StatusCode::NO_CONTENT)
}
