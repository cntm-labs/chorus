use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use crate::app::AppState;
use crate::auth::admin::AdminContext;

/// Billing account summary combining account, subscription, and usage data.
#[derive(Serialize, sqlx::FromRow)]
pub struct BillingAccountSummary {
    pub account_id: Uuid,
    pub account_name: String,
    pub plan_slug: String,
    pub status: String,
    pub sms_sent: i32,
    pub sms_quota: i32,
    pub email_sent: i32,
    pub email_quota: i32,
    pub period_end: chrono::DateTime<chrono::Utc>,
}

/// Billing report with revenue and plan distribution.
#[derive(Serialize)]
pub struct BillingReport {
    pub total_revenue_cents: i64,
    pub accounts_by_plan: Vec<PlanCount>,
    pub overage_accounts: Vec<Uuid>,
}

/// Plan distribution count.
#[derive(Serialize, sqlx::FromRow)]
pub struct PlanCount {
    pub plan_slug: String,
    pub count: i64,
}

/// Request body for overriding an account's plan.
#[derive(Deserialize)]
pub struct OverridePlanRequest {
    pub plan_slug: String,
}

/// Request body for adjusting usage counters.
#[derive(Deserialize)]
pub struct AdjustUsageRequest {
    pub sms_delta: Option<i32>,
    pub email_delta: Option<i32>,
}

/// `GET /admin/billing/accounts` — list all accounts with billing status.
pub async fn list_accounts(
    State(state): State<Arc<AppState>>,
    _admin: AdminContext,
) -> Result<Json<Vec<BillingAccountSummary>>, (StatusCode, String)> {
    let accounts = state
        .admin_repo()
        .list_billing_accounts()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(accounts))
}

/// `PATCH /admin/billing/accounts/{id}/plan` — override subscription plan.
pub async fn override_plan(
    State(state): State<Arc<AppState>>,
    _admin: AdminContext,
    Path(id): Path<Uuid>,
    Json(body): Json<OverridePlanRequest>,
) -> Result<StatusCode, (StatusCode, String)> {
    state
        .admin_repo()
        .override_plan(id, &body.plan_slug)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(StatusCode::NO_CONTENT)
}

/// `PATCH /admin/billing/accounts/{id}/usage` — adjust usage counters.
pub async fn adjust_usage(
    State(state): State<Arc<AppState>>,
    _admin: AdminContext,
    Path(id): Path<Uuid>,
    Json(body): Json<AdjustUsageRequest>,
) -> Result<StatusCode, (StatusCode, String)> {
    state
        .admin_repo()
        .adjust_usage(id, body.sms_delta, body.email_delta)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(StatusCode::NO_CONTENT)
}

/// `GET /admin/billing/reports` — billing report.
pub async fn report(
    State(state): State<Arc<AppState>>,
    _admin: AdminContext,
) -> Result<Json<BillingReport>, (StatusCode, String)> {
    let report = state
        .admin_repo()
        .billing_report()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(report))
}
