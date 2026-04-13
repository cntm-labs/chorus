use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use serde::Serialize;
use std::sync::Arc;

use crate::app::AppState;

/// Health check response.
#[derive(Serialize)]
pub struct HealthResponse {
    pub status: &'static str,
    pub version: &'static str,
}

/// Returns server health status after verifying database and Redis connectivity.
pub async fn health(
    State(state): State<Arc<AppState>>,
) -> Result<Json<HealthResponse>, (StatusCode, &'static str)> {
    // Verify database connectivity (if pool is available)
    if let Some(db) = &state.db {
        sqlx::query("SELECT 1")
            .execute(db)
            .await
            .map_err(|_| (StatusCode::SERVICE_UNAVAILABLE, "database unavailable"))?;
    }

    // Verify Redis connectivity
    let mut conn = state
        .redis
        .get_multiplexed_tokio_connection()
        .await
        .map_err(|_| (StatusCode::SERVICE_UNAVAILABLE, "redis unavailable"))?;
    redis::cmd("PING")
        .query_async::<String>(&mut conn)
        .await
        .map_err(|_| (StatusCode::SERVICE_UNAVAILABLE, "redis unavailable"))?;

    Ok(Json(HealthResponse {
        status: "ok",
        version: env!("CARGO_PKG_VERSION"),
    }))
}
