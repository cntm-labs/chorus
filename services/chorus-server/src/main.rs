use chorus_server::app::{create_router, AppState};
use chorus_server::config::Config;
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
    let config = Arc::new(config);
    let state = AppState::new(db, redis, Arc::clone(&config));
    let state = Arc::new(state);

    // Spawn background queue workers and delayed poller
    chorus_server::queue::worker::spawn_workers(
        Arc::clone(&state),
        Arc::clone(&config),
        config.worker_concurrency,
    );
    chorus_server::queue::delayed::spawn_delayed_poller(state.redis.clone());
    chorus_server::queue::webhook_dispatch::spawn_webhook_retry_poller(
        state.redis.clone(),
        state.http_client().clone(),
    );

    let app = create_router(state);

    let addr = format!("{}:{}", config.host, config.port);
    tracing::info!("chorus-server listening on {}", addr);

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
