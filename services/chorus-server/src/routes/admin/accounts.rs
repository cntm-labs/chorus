use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use crate::app::AppState;
use crate::auth::admin::AdminContext;

/// Account summary for list view.
#[derive(Serialize, sqlx::FromRow)]
pub struct AccountListItem {
    pub id: Uuid,
    pub name: String,
    pub owner_email: String,
    pub is_active: bool,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// Account detail with usage stats.
#[derive(Serialize, sqlx::FromRow)]
pub struct AccountDetail {
    pub id: Uuid,
    pub name: String,
    pub owner_email: String,
    pub is_active: bool,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    pub key_count: i64,
    pub message_count: i64,
}

/// `GET /admin/accounts` — list all accounts.
pub async fn list(
    State(state): State<Arc<AppState>>,
    _admin: AdminContext,
) -> Result<Json<Vec<AccountListItem>>, (StatusCode, String)> {
    let accounts = state
        .admin_repo()
        .list_accounts()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(accounts))
}

/// `GET /admin/accounts/{id}` — account detail with usage stats.
pub async fn detail(
    State(state): State<Arc<AppState>>,
    _admin: AdminContext,
    Path(id): Path<Uuid>,
) -> Result<Json<AccountDetail>, (StatusCode, String)> {
    let account = state
        .admin_repo()
        .get_account_detail(id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "account not found".into()))?;

    Ok(Json(account))
}

/// Request body for creating an account.
#[derive(Deserialize)]
pub struct CreateAccountRequest {
    pub name: String,
    pub owner_email: String,
}

/// `POST /admin/accounts` — create a new account.
pub async fn create(
    State(state): State<Arc<AppState>>,
    _admin: AdminContext,
    Json(body): Json<CreateAccountRequest>,
) -> Result<(StatusCode, Json<AccountListItem>), (StatusCode, String)> {
    let account = state
        .admin_repo()
        .create_account(&body.name, &body.owner_email)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok((StatusCode::CREATED, Json(account)))
}

/// Request body for updating an account.
#[derive(Deserialize)]
pub struct UpdateAccountRequest {
    pub is_active: Option<bool>,
    pub name: Option<String>,
}

/// `PATCH /admin/accounts/{id}` — update account fields.
pub async fn update(
    State(state): State<Arc<AppState>>,
    _admin: AdminContext,
    Path(id): Path<Uuid>,
    Json(body): Json<UpdateAccountRequest>,
) -> Result<StatusCode, (StatusCode, String)> {
    state
        .admin_repo()
        .update_account(id, body.is_active, body.name.as_deref())
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(StatusCode::NO_CONTENT)
}

/// `DELETE /admin/accounts/{id}` — soft-delete account.
pub async fn soft_delete(
    State(state): State<Arc<AppState>>,
    _admin: AdminContext,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, (StatusCode, String)> {
    state
        .admin_repo()
        .deactivate_account(id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(StatusCode::NO_CONTENT)
}
