mod app;
mod auth;
mod config;
mod db;
mod queue;
mod routes;

use app::{AppState, create_router};
use config::Config;
use std::sync::Arc;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "chorus_server=debug,tower_http=debug".into()),
        )
        .init();

    let config = Config::from_env();

    let db = sqlx::PgPool::connect(&config.database_url).await?;
    sqlx::migrate!("src/db/migrations").run(&db).await?;

    let redis = redis::Client::open(config.redis_url.as_str())?;

    let state = AppState::new(db, redis);
    let state = Arc::new(state);

    // Spawn background queue worker
    queue::worker::spawn_worker(Arc::clone(&state));

    let app = create_router(state);

    let addr = format!("{}:{}", config.host, config.port);
    tracing::info!("chorus-server listening on {}", addr);

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
