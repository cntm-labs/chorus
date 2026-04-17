use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use crate::app::AppState;
use crate::auth::admin::AdminContext;

/// Provider config summary for admin list view.
#[derive(Serialize, sqlx::FromRow)]
pub struct AdminProviderConfig {
    pub id: Uuid,
    pub account_id: Uuid,
    pub channel: String,
    pub provider: String,
    pub priority: i32,
    pub is_active: bool,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// Provider health summary from recent delivery data.
#[derive(Serialize, sqlx::FromRow)]
pub struct ProviderHealth {
    pub id: Uuid,
    pub provider: String,
    pub total_sent: i64,
    pub total_errors: i64,
    pub error_rate: f64,
    pub last_success: Option<chrono::DateTime<chrono::Utc>>,
    pub last_error: Option<chrono::DateTime<chrono::Utc>>,
}

/// `GET /admin/providers` — list all provider configs across accounts.
pub async fn list_all(
    State(state): State<Arc<AppState>>,
    _admin: AdminContext,
) -> Result<Json<Vec<AdminProviderConfig>>, (StatusCode, String)> {
    let configs = state
        .admin_repo()
        .list_all_provider_configs()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(configs))
}

/// `GET /admin/providers/{id}/health` — provider health summary.
pub async fn health(
    State(state): State<Arc<AppState>>,
    _admin: AdminContext,
    Path(id): Path<Uuid>,
) -> Result<Json<ProviderHealth>, (StatusCode, String)> {
    let health = state
        .admin_repo()
        .get_provider_health(id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "provider config not found".into()))?;

    Ok(Json(health))
}

/// Request body for updating a provider config.
#[derive(Deserialize)]
pub struct UpdateProviderRequest {
    pub priority: Option<i32>,
    pub is_active: Option<bool>,
}

/// `PATCH /admin/providers/{id}` — update provider config.
pub async fn update(
    State(state): State<Arc<AppState>>,
    _admin: AdminContext,
    Path(id): Path<Uuid>,
    Json(body): Json<UpdateProviderRequest>,
) -> Result<StatusCode, (StatusCode, String)> {
    state
        .admin_repo()
        .update_provider_config(id, body.priority, body.is_active)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(StatusCode::NO_CONTENT)
}

/// Request body for bulk disabling a provider.
#[derive(Deserialize)]
pub struct BulkDisableRequest {
    pub provider: String,
}

/// Response for bulk disable operation.
#[derive(Serialize)]
pub struct BulkDisableResponse {
    pub affected: u64,
}

/// `POST /admin/providers/disable` — disable a provider across all accounts.
pub async fn bulk_disable(
    State(state): State<Arc<AppState>>,
    _admin: AdminContext,
    Json(body): Json<BulkDisableRequest>,
) -> Result<Json<BulkDisableResponse>, (StatusCode, String)> {
    let affected = state
        .admin_repo()
        .disable_provider_by_name(&body.provider)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(BulkDisableResponse { affected }))
}
