use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::app::AppState;
use crate::auth::api_key::AccountContext;
use crate::billing::stripe_client::StripeClient;
use crate::db::billing::{BillingPlan, Subscription, Usage};

/// Response for current plan + usage.
#[derive(Serialize)]
pub struct PlanUsageResponse {
    pub plan: Option<BillingPlan>,
    pub subscription: Option<Subscription>,
    pub usage: Option<Usage>,
}

/// Response for listing available plans.
#[derive(Serialize)]
pub struct PlansResponse {
    pub plans: Vec<BillingPlan>,
}

/// Request for creating a Stripe checkout session.
#[derive(Deserialize)]
pub struct CheckoutRequest {
    pub plan_slug: String,
    pub success_url: String,
    pub cancel_url: String,
}

/// Response with Stripe checkout URL.
#[derive(Serialize)]
pub struct CheckoutResponse {
    pub checkout_url: String,
}

/// GET /v1/billing/plans — list available billing plans.
pub async fn list_plans(
    State(state): State<Arc<AppState>>,
    _ctx: AccountContext,
) -> Result<Json<PlansResponse>, (StatusCode, String)> {
    let plans = state
        .billing_repo()
        .list_plans()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(PlansResponse { plans }))
}

/// GET /v1/billing/plan — current plan, subscription, and usage.
pub async fn get_plan(
    State(state): State<Arc<AppState>>,
    ctx: AccountContext,
) -> Result<Json<PlanUsageResponse>, (StatusCode, String)> {
    let billing = state.billing_repo();

    let subscription = billing
        .get_subscription(ctx.account_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let plan = if let Some(ref sub) = subscription {
        billing
            .get_plan_by_slug(&format!("{}", sub.plan_id))
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
    } else {
        None
    };

    let usage = billing
        .get_usage(ctx.account_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(PlanUsageResponse {
        plan,
        subscription,
        usage,
    }))
}

/// POST /v1/billing/checkout — create a Stripe checkout session.
pub async fn create_checkout(
    State(state): State<Arc<AppState>>,
    ctx: AccountContext,
    Json(req): Json<CheckoutRequest>,
) -> Result<(StatusCode, Json<CheckoutResponse>), (StatusCode, String)> {
    let stripe_key = state.config().stripe_secret_key.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "billing not configured".into(),
    ))?;

    let billing = state.billing_repo();

    let plan = billing
        .get_plan_by_slug(&req.plan_slug)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::BAD_REQUEST, "plan not found".into()))?;

    if plan.price_cents == 0 {
        return Err((StatusCode::BAD_REQUEST, "cannot checkout free plan".into()));
    }

    let stripe = StripeClient::new(stripe_key);

    // Get or create Stripe customer
    let sub = billing
        .get_subscription(ctx.account_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let customer_id = if let Some(ref s) = sub {
        s.stripe_customer_id.clone().unwrap_or_default()
    } else {
        let customer = stripe
            .create_customer(&ctx.account_email, &ctx.account_name)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        customer.id.to_string()
    };

    let session = stripe
        .create_checkout_session(
            &customer_id,
            &plan.name,
            plan.price_cents.into(),
            &req.success_url,
            &req.cancel_url,
        )
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let url = session.url.unwrap_or_default();

    Ok((StatusCode::OK, Json(CheckoutResponse { checkout_url: url })))
}

/// GET /v1/billing/usage — current period usage.
pub async fn get_usage(
    State(state): State<Arc<AppState>>,
    ctx: AccountContext,
) -> Result<Json<Option<Usage>>, (StatusCode, String)> {
    let usage = state
        .billing_repo()
        .get_usage(ctx.account_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(usage))
}
