pub mod accounts;
pub mod providers;

use axum::routing::get;
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
        .route("/providers/disable", axum::routing::post(providers::bulk_disable))
}
