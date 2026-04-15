use axum::extract::State;
use axum::Json;
use serde::Serialize;
use std::sync::Arc;

use crate::app::AppState;

/// Health check response with component details.
#[derive(Serialize)]
pub struct HealthResponse {
    pub status: &'static str,
    pub version: &'static str,
    pub components: Components,
}

/// Individual component health status.
#[derive(Serialize)]
pub struct Components {
    pub database: ComponentStatus,
    pub redis: ComponentStatus,
    pub queue_depth: i64,
}

/// Status of a single component.
#[derive(Serialize)]
pub struct ComponentStatus {
    pub status: &'static str,
}

/// Returns server health status after verifying database and Redis connectivity.
pub async fn health(State(state): State<Arc<AppState>>) -> Json<HealthResponse> {
    let db_status = check_database(&state).await;
    let redis_status = check_redis(&state).await;
    let queue_depth = get_queue_depth(&state).await;

    let overall = if db_status == "ok" && redis_status == "ok" {
        "ok"
    } else {
        "degraded"
    };

    // Report queue depth as a gauge metric
    metrics::gauge!("chorus_queue_depth").set(queue_depth as f64);

    Json(HealthResponse {
        status: overall,
        version: env!("CARGO_PKG_VERSION"),
        components: Components {
            database: ComponentStatus { status: db_status },
            redis: ComponentStatus {
                status: redis_status,
            },
            queue_depth,
        },
    })
}

async fn check_database(state: &AppState) -> &'static str {
    if let Some(db) = &state.db {
        match sqlx::query("SELECT 1").execute(db).await {
            Ok(_) => "ok",
            Err(_) => "unavailable",
        }
    } else {
        "not_configured"
    }
}

async fn check_redis(state: &AppState) -> &'static str {
    let conn = state.redis.get_multiplexed_tokio_connection().await;
    match conn {
        Ok(mut c) => match redis::cmd("PING").query_async::<String>(&mut c).await {
            Ok(_) => "ok",
            Err(_) => "unavailable",
        },
        Err(_) => "unavailable",
    }
}

async fn get_queue_depth(state: &AppState) -> i64 {
    let conn = state.redis.get_multiplexed_tokio_connection().await;
    match conn {
        Ok(mut c) => redis::cmd("LLEN")
            .arg(crate::queue::QUEUE_KEY)
            .query_async::<i64>(&mut c)
            .await
            .unwrap_or(0),
        Err(_) => -1,
    }
}
