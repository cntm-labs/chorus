use axum::routing::{delete, get, post};
use axum::Router;
use sqlx::PgPool;
use std::sync::Arc;

use crate::db::postgres::PgRepository;
use crate::db::provider_config::PgProviderConfigRepository;
use crate::db::{AccountRepository, ApiKeyRepository, MessageRepository, ProviderConfigRepository};
use crate::routes;

/// Shared application state accessible to all request handlers.
pub struct AppState {
    /// PostgreSQL connection pool (used by health check and migrations).
    pub db: Option<PgPool>,
    /// Redis client for queue, caching, and OTP.
    pub redis: redis::Client,
    /// Account + API key repository.
    account_repo: Arc<dyn AccountRepository>,
    /// Message repository.
    message_repo: Arc<dyn MessageRepository>,
    /// API key management repository.
    api_key_repo: Arc<dyn ApiKeyRepository>,
    /// Provider config repository.
    provider_config_repo: Arc<dyn ProviderConfigRepository>,
}

impl AppState {
    /// Create app state backed by PostgreSQL.
    pub fn new(db: PgPool, redis: redis::Client) -> Self {
        let repo = Arc::new(PgRepository::new(db.clone()));
        let provider_config_repo = Arc::new(PgProviderConfigRepository::new(db.clone()));
        Self {
            db: Some(db),
            redis,
            account_repo: repo.clone(),
            message_repo: repo.clone(),
            api_key_repo: repo,
            provider_config_repo,
        }
    }

    /// Create app state with custom repositories (for testing).
    pub fn with_repos(
        redis: redis::Client,
        account_repo: Arc<dyn AccountRepository>,
        message_repo: Arc<dyn MessageRepository>,
        api_key_repo: Arc<dyn ApiKeyRepository>,
        provider_config_repo: Arc<dyn ProviderConfigRepository>,
    ) -> Self {
        Self {
            db: None,
            redis,
            account_repo,
            message_repo,
            api_key_repo,
            provider_config_repo,
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

    /// Access the provider config repository.
    pub fn provider_config_repo(&self) -> Arc<dyn ProviderConfigRepository> {
        Arc::clone(&self.provider_config_repo)
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
        .route(
            "/v1/keys",
            get(routes::keys::list_keys).post(routes::keys::create_key),
        )
        .route("/v1/keys/{id}", delete(routes::keys::revoke_key))
        .route("/v1/otp/send", post(routes::otp::send_otp))
        .route("/v1/otp/verify", post(routes::otp::verify_otp))
        .route(
            "/v1/providers",
            get(routes::provider_configs::list_provider_configs)
                .post(routes::provider_configs::create_provider_config),
        )
        .route(
            "/v1/providers/{id}",
            delete(routes::provider_configs::delete_provider_config),
        )
        .with_state(state)
}
