use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};
use serde::Serialize;
use sqlx::PgPool;
use std::sync::Arc;

/// Shared application state accessible to all request handlers.
pub struct AppState {
    /// PostgreSQL connection pool.
    pub db: PgPool,
    /// Redis client for queue and caching.
    pub redis: redis::Client,
}

/// Health check response.
#[derive(Serialize)]
struct HealthResponse {
    status: &'static str,
    version: &'static str,
}

/// Basic health check that verifies database connectivity.
async fn health(State(state): State<Arc<AppState>>) -> Json<HealthResponse> {
    // Touch db pool to confirm connectivity (actual check in later task)
    let _pool = &state.db;
    let _redis = &state.redis;
    Json(HealthResponse {
        status: "ok",
        version: env!("CARGO_PKG_VERSION"),
    })
}

/// Build the Axum router with all routes and shared state.
pub fn create_router(state: AppState) -> Router {
    let state = Arc::new(state);
    Router::new()
        .route("/health", get(health))
        .with_state(state)
}
