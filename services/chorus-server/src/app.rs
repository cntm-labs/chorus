use axum::routing::{delete, get, post};
use axum::Router;
use sqlx::PgPool;
use std::sync::Arc;

use crate::config::Config;
use crate::db::billing::{BillingRepository, PgBillingRepository};
use crate::db::postgres::PgRepository;
use crate::db::provider_config::PgProviderConfigRepository;
use crate::db::webhook::PgWebhookRepository;
use crate::db::{
    AccountRepository, ApiKeyRepository, MessageRepository, ProviderConfigRepository,
    WebhookRepository,
};
use crate::routes;

/// Shared application state accessible to all request handlers.
pub struct AppState {
    /// PostgreSQL connection pool (used by health check and migrations).
    pub db: Option<PgPool>,
    /// Redis client for queue, caching, and OTP.
    pub redis: redis::Client,
    /// Shared HTTP client for outbound requests (webhooks, etc.).
    http_client: reqwest::Client,
    /// Server configuration.
    config: Arc<Config>,
    /// Account + API key repository.
    account_repo: Arc<dyn AccountRepository>,
    /// Message repository.
    message_repo: Arc<dyn MessageRepository>,
    /// API key management repository.
    api_key_repo: Arc<dyn ApiKeyRepository>,
    /// Provider config repository.
    provider_config_repo: Arc<dyn ProviderConfigRepository>,
    /// Webhook repository.
    webhook_repo: Arc<dyn WebhookRepository>,
    /// Billing repository.
    billing_repo: Arc<dyn BillingRepository>,
}

impl AppState {
    /// Create app state backed by PostgreSQL.
    pub fn new(db: PgPool, redis: redis::Client, config: Arc<Config>) -> Self {
        let repo = Arc::new(PgRepository::new(db.clone()));
        let provider_config_repo = Arc::new(PgProviderConfigRepository::new(db.clone()));
        let webhook_repo = Arc::new(PgWebhookRepository::new(db.clone()));
        let billing_repo = Arc::new(PgBillingRepository::new(db.clone()));
        Self {
            db: Some(db),
            redis,
            http_client: reqwest::Client::new(),
            config,
            account_repo: repo.clone(),
            message_repo: repo.clone(),
            api_key_repo: repo,
            provider_config_repo,
            webhook_repo,
            billing_repo,
        }
    }

    /// Create app state with custom repositories (for testing).
    pub fn with_repos(
        redis: redis::Client,
        config: Arc<Config>,
        account_repo: Arc<dyn AccountRepository>,
        message_repo: Arc<dyn MessageRepository>,
        api_key_repo: Arc<dyn ApiKeyRepository>,
        provider_config_repo: Arc<dyn ProviderConfigRepository>,
        webhook_repo: Arc<dyn WebhookRepository>,
    ) -> Self {
        Self {
            db: None,
            redis,
            http_client: reqwest::Client::new(),
            config,
            account_repo,
            message_repo,
            api_key_repo,
            provider_config_repo,
            webhook_repo,
            billing_repo: Arc::new(crate::db::billing::NullBillingRepository),
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

    /// Access the webhook repository.
    pub fn webhook_repo(&self) -> Arc<dyn WebhookRepository> {
        Arc::clone(&self.webhook_repo)
    }

    /// Access the billing repository.
    pub fn billing_repo(&self) -> Arc<dyn BillingRepository> {
        Arc::clone(&self.billing_repo)
    }

    /// Access the shared HTTP client.
    pub fn http_client(&self) -> &reqwest::Client {
        &self.http_client
    }

    /// Access the server configuration.
    pub fn config(&self) -> &Config {
        &self.config
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
        .route(
            "/v1/webhooks",
            get(routes::webhooks::list_webhooks).post(routes::webhooks::create_webhook),
        )
        .route(
            "/v1/webhooks/{id}",
            delete(routes::webhooks::delete_webhook),
        )
        .route("/v1/sms/send-batch", post(routes::batch::send_sms_batch))
        .route(
            "/v1/email/send-batch",
            post(routes::batch::send_email_batch),
        )
        .route("/v1/billing/plans", get(routes::billing::list_plans))
        .route("/v1/billing/plan", get(routes::billing::get_plan))
        .route("/v1/billing/checkout", post(routes::billing::create_checkout))
        .route("/v1/billing/usage", get(routes::billing::get_usage))
        .route("/internal/bounces", post(routes::internal::handle_bounce))
        .route("/internal/dns-check", get(routes::internal::dns_check))
        .with_state(state)
}
