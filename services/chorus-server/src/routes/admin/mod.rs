pub mod accounts;
pub mod billing;
pub mod dlq;
pub mod messages;
pub mod providers;
pub mod webhooks;

use axum::routing::{delete, get, post};
use axum::Router;
use std::sync::Arc;

use crate::app::AppState;

/// Build the admin sub-router with all admin endpoints.
pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        // Account Management (#34)
        .route("/accounts", get(accounts::list).post(accounts::create))
        .route(
            "/accounts/{id}",
            get(accounts::detail)
                .patch(accounts::update)
                .delete(accounts::soft_delete),
        )
        // Provider Config (#35)
        .route("/providers", get(providers::list_all))
        .route("/providers/{id}/health", get(providers::health))
        .route("/providers/{id}", axum::routing::patch(providers::update))
        .route("/providers/disable", post(providers::bulk_disable))
        // DLQ Management (#36)
        .route("/dlq", get(dlq::list))
        .route(
            "/dlq/{message_id}",
            get(dlq::detail).delete(dlq::purge_single),
        )
        .route("/dlq/{message_id}/retry", post(dlq::retry_single))
        .route("/dlq/retry-batch", post(dlq::retry_batch))
        .route("/dlq/purge", delete(dlq::purge_all))
        // Message Inspector (#37)
        .route("/messages", get(messages::search))
        .route("/messages/{id}", get(messages::detail))
        // Billing (#38)
        .route("/billing/accounts", get(billing::list_accounts))
        .route(
            "/billing/accounts/{id}/plan",
            axum::routing::patch(billing::override_plan),
        )
        .route(
            "/billing/accounts/{id}/usage",
            axum::routing::patch(billing::adjust_usage),
        )
        .route("/billing/reports", get(billing::report))
        // Webhook Admin (#39)
        .route("/webhooks", get(webhooks::list_all))
        .route(
            "/webhooks/{id}",
            axum::routing::patch(webhooks::update_status),
        )
        .route("/webhooks/{id}/deliveries", get(webhooks::deliveries))
        .route("/webhooks/{id}/test", post(webhooks::test_webhook))
        .route(
            "/webhooks/disable-account/{account_id}",
            post(webhooks::disable_account),
        )
}
