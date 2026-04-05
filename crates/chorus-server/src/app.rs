use axum::routing::{delete, get, post};
use axum::Router;
use sqlx::PgPool;
use std::sync::Arc;

use crate::db::postgres::PgRepository;
use crate::db::{AccountRepository, ApiKeyRepository, MessageRepository};
use crate::routes;

/// Shared application state accessible to all request handlers.
pub struct AppState {
    /// PostgreSQL connection pool.
    pub db: PgPool,
    /// Redis client for queue and caching.
    pub redis: redis::Client,
    /// Account + API key repository.
    account_repo: Arc<dyn AccountRepository>,
    /// Message repository.
    message_repo: Arc<dyn MessageRepository>,
    /// API key management repository.
    api_key_repo: Arc<dyn ApiKeyRepository>,
}

impl AppState {
    /// Create app state from database pool and redis client.
    pub fn new(db: PgPool, redis: redis::Client) -> Self {
        let repo = Arc::new(PgRepository::new(db.clone()));
        Self {
            db,
            redis,
            account_repo: repo.clone(),
            message_repo: repo.clone(),
            api_key_repo: repo,
        }
    }

    /// Access the account repository.
    pub fn account_repo(&self) -> Arc<dyn AccountRepository> {
        Arc::clone(&self.account_repo)
    }

    /// Access the message repository.
    pub fn message_repo(&self) -> Arc<dyn MessageRepository> {
        Arc::clone(&self.message_repo)
    }

    /// Access the API key repository.
    pub fn api_key_repo(&self) -> Arc<dyn ApiKeyRepository> {
        Arc::clone(&self.api_key_repo)
    }
}

/// Build the Axum router with all routes and shared state.
pub fn create_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/health", get(routes::health::health))
        .route("/v1/sms/send", post(routes::sms::send_sms))
        .route("/v1/email/send", post(routes::email::send_email))
        .route("/v1/messages", get(routes::messages::list_messages))
        .route("/v1/messages/{id}", get(routes::messages::get_message))
        .route("/v1/keys", get(routes::keys::list_keys).post(routes::keys::create_key))
        .route("/v1/keys/{id}", delete(routes::keys::revoke_key))
        .route("/v1/otp/send", post(routes::otp::send_otp))
        .route("/v1/otp/verify", post(routes::otp::verify_otp))
        .with_state(state)
}
