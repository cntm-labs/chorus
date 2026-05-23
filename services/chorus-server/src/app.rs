use axum::middleware as axum_middleware;
use axum::routing::{delete, get, post};
use axum::Router;
use metrics_exporter_prometheus::PrometheusHandle;
use sqlx::PgPool;
use std::sync::Arc;

use crate::config::Config;
use crate::db::billing::{BillingRepository, PgBillingRepository};
use crate::db::idempotency::PgIdempotencyRepository;
use crate::db::postgres::PgRepository;
use crate::db::provider_config::PgProviderConfigRepository;
use crate::db::suppression::PgSuppressionRepository;
use crate::db::verification::PgVerificationRepository;
use crate::db::webhook::PgWebhookRepository;
use crate::crypto::Encryptor;
use crate::db::totp::PgTotpRepository;
use crate::db::{
    AccountRepository, AdminKeyRepository, AdminRepository, ApiKeyRepository,
    IdempotencyRepository, MessageRepository, PgAdminRepository, ProviderConfigRepository,
    SuppressionRepository, TotpRepository, VerificationRepository, WebhookRepository,
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
    /// Suppression list repository.
    suppression_repo: Arc<dyn SuppressionRepository>,
    /// Idempotency record repository.
    idempotency_repo: Arc<dyn IdempotencyRepository>,
    /// Verification record repository.
    verification_repo: Arc<dyn VerificationRepository>,
    /// TOTP record repository.
    totp_repo: Arc<dyn TotpRepository>,
    /// AES-GCM encryptor for secret-at-rest columns.
    encryptor: Arc<Encryptor>,
    /// Billing repository.
    billing_repo: Arc<dyn BillingRepository>,
    /// Admin key repository.
    admin_key_repo: Arc<dyn AdminKeyRepository>,
    /// Admin repository for cross-account queries.
    admin_repo: Arc<dyn AdminRepository>,
}

impl AppState {
    /// Create app state backed by PostgreSQL.
    pub fn new(db: PgPool, redis: redis::Client, config: Arc<Config>) -> Self {
        let repo = Arc::new(PgRepository::new(db.clone()));
        let provider_config_repo = Arc::new(PgProviderConfigRepository::new(db.clone()));
        let webhook_repo = Arc::new(PgWebhookRepository::new(db.clone()));
        let suppression_repo = Arc::new(PgSuppressionRepository::new(db.clone()));
        let idempotency_repo = Arc::new(PgIdempotencyRepository::new(db.clone()));
        let verification_repo = Arc::new(PgVerificationRepository::new(db.clone()));
        let totp_repo = Arc::new(PgTotpRepository::new(db.clone()));
        let encryptor = Arc::new(
            Encryptor::from_env().expect("CHORUS_ENCRYPTION_KEY missing or invalid")
        );
        let billing_repo = Arc::new(PgBillingRepository::new(db.clone()));
        let admin_repo = Arc::new(PgAdminRepository::new(db.clone()));
        Self {
            db: Some(db),
            redis,
            http_client: reqwest::Client::new(),
            config,
            account_repo: repo.clone(),
            message_repo: repo.clone(),
            api_key_repo: repo.clone(),
            admin_key_repo: repo,
            provider_config_repo,
            webhook_repo,
            suppression_repo,
            idempotency_repo,
            verification_repo,
            totp_repo,
            encryptor,
            billing_repo,
            admin_repo,
        }
    }

    /// Create app state with custom repositories (for testing).
    #[allow(clippy::too_many_arguments)]
    pub fn with_repos(
        redis: redis::Client,
        config: Arc<Config>,
        account_repo: Arc<dyn AccountRepository>,
        message_repo: Arc<dyn MessageRepository>,
        api_key_repo: Arc<dyn ApiKeyRepository>,
        provider_config_repo: Arc<dyn ProviderConfigRepository>,
        webhook_repo: Arc<dyn WebhookRepository>,
        suppression_repo: Arc<dyn SuppressionRepository>,
        idempotency_repo: Arc<dyn IdempotencyRepository>,
        verification_repo: Arc<dyn VerificationRepository>,
        totp_repo: Arc<dyn TotpRepository>,
        encryptor: Arc<Encryptor>,
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
            suppression_repo,
            idempotency_repo,
            verification_repo,
            totp_repo,
            encryptor,
            billing_repo: Arc::new(crate::db::billing::NullBillingRepository),
            admin_key_repo: Arc::new(NullAdminKeyRepository),
            admin_repo: Arc::new(NullAdminRepository),
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

    /// Access the suppression repository.
    pub fn suppression_repo(&self) -> Arc<dyn SuppressionRepository> {
        Arc::clone(&self.suppression_repo)
    }

    /// Access the idempotency repository.
    pub fn idempotency_repo(&self) -> Arc<dyn IdempotencyRepository> {
        Arc::clone(&self.idempotency_repo)
    }

    /// Access the verification repository.
    pub fn verification_repo(&self) -> Arc<dyn VerificationRepository> {
        Arc::clone(&self.verification_repo)
    }

    /// Access the TOTP repository.
    pub fn totp_repo(&self) -> Arc<dyn TotpRepository> {
        Arc::clone(&self.totp_repo)
    }

    /// Access the encryptor (AES-GCM keyed at startup).
    pub fn encryptor(&self) -> &Arc<Encryptor> {
        &self.encryptor
    }

    /// Access the billing repository.
    pub fn billing_repo(&self) -> Arc<dyn BillingRepository> {
        Arc::clone(&self.billing_repo)
    }

    /// Access the admin key repository.
    pub fn admin_key_repo(&self) -> Arc<dyn AdminKeyRepository> {
        Arc::clone(&self.admin_key_repo)
    }

    /// Access the admin repository.
    pub fn admin_repo(&self) -> Arc<dyn AdminRepository> {
        Arc::clone(&self.admin_repo)
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
    create_router_with_metrics(state, None)
}

/// Build the Axum router with optional Prometheus metrics handle.
pub fn create_router_with_metrics(
    state: Arc<AppState>,
    metrics_handle: Option<PrometheusHandle>,
) -> Router {
    let mut router = Router::new()
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
        .route(
            "/v1/suppressions",
            get(routes::suppressions::list_suppressions)
                .post(routes::suppressions::create_suppression),
        )
        .route(
            "/v1/suppressions/{channel}/{recipient}",
            delete(routes::suppressions::delete_suppression),
        )
        .route("/v1/sms/send-batch", post(routes::batch::send_sms_batch))
        .route(
            "/v1/email/send-batch",
            post(routes::batch::send_email_batch),
        )
        .route("/v1/billing/plans", get(routes::billing::list_plans))
        .route("/v1/billing/plan", get(routes::billing::get_plan))
        .route(
            "/v1/billing/checkout",
            post(routes::billing::create_checkout),
        )
        .route("/v1/billing/usage", get(routes::billing::get_usage))
        .route(
            "/v1/verifications",
            post(routes::verifications::create_verification)
                .get(routes::verifications::list_verifications),
        )
        .route(
            "/v1/verifications/{id}",
            get(routes::verifications::get_verification),
        )
        .route(
            "/v1/verifications/{id}/check",
            post(routes::verifications::check_verification),
        )
        .route(
            "/v1/verifications/{id}/cancel",
            post(routes::verifications::cancel_verification),
        )
        .route(
            "/v1/verifications/{id}/resend",
            post(routes::verifications::resend_verification),
        )
        .route("/internal/bounces", post(routes::internal::handle_bounce))
        .route("/internal/dns-check", get(routes::internal::dns_check))
        .nest("/admin", routes::admin::router())
        .route("/v1/totp/enroll", post(routes::totp::enroll_totp))
        .route("/v1/totp/activate", post(routes::totp::activate_totp))
        .route("/v1/totp/verify", post(routes::totp::verify_totp))
        .route(
            "/v1/totp/{user_id}",
            get(routes::totp::get_totp_status).delete(routes::totp::disenroll_totp),
        )
        .route(
            "/v1/totp/backup-codes/regenerate",
            post(routes::totp::regenerate_backup_codes),
        )
        .route("/v1/totp/{user_id}/qr", get(routes::totp::get_totp_qr))
        .with_state(state)
        .layer(axum_middleware::from_fn(crate::middleware::metrics::track))
        .layer(axum_middleware::from_fn(
            crate::middleware::request_id::inject,
        ));

    if let Some(handle) = metrics_handle {
        router = router.merge(
            Router::new()
                .route("/metrics", get(crate::metrics::handler))
                .with_state(handle),
        );
    }

    router
}

/// No-op admin key repository for tests.
struct NullAdminKeyRepository;

/// No-op admin repository for tests.
struct NullAdminRepository;

#[async_trait::async_trait]
impl AdminKeyRepository for NullAdminKeyRepository {
    async fn find_by_hash(
        &self,
        _hash: &str,
    ) -> Result<Option<crate::db::AdminKey>, crate::db::DbError> {
        Ok(None)
    }
}

#[async_trait::async_trait]
impl AdminRepository for NullAdminRepository {
    async fn list_accounts(
        &self,
    ) -> Result<Vec<crate::routes::admin::accounts::AccountListItem>, crate::db::DbError> {
        Ok(vec![])
    }
    async fn get_account_detail(
        &self,
        _id: uuid::Uuid,
    ) -> Result<Option<crate::routes::admin::accounts::AccountDetail>, crate::db::DbError> {
        Ok(None)
    }
    async fn create_account(
        &self,
        _name: &str,
        _email: &str,
    ) -> Result<crate::routes::admin::accounts::AccountListItem, crate::db::DbError> {
        Err(crate::db::DbError::Internal(anyhow::anyhow!(
            "not implemented"
        )))
    }
    async fn update_account(
        &self,
        _id: uuid::Uuid,
        _is_active: Option<bool>,
        _name: Option<&str>,
    ) -> Result<(), crate::db::DbError> {
        Ok(())
    }
    async fn deactivate_account(&self, _id: uuid::Uuid) -> Result<(), crate::db::DbError> {
        Ok(())
    }
    async fn list_all_provider_configs(
        &self,
    ) -> Result<Vec<crate::routes::admin::providers::AdminProviderConfig>, crate::db::DbError> {
        Ok(vec![])
    }
    async fn get_provider_health(
        &self,
        _id: uuid::Uuid,
    ) -> Result<Option<crate::routes::admin::providers::ProviderHealth>, crate::db::DbError> {
        Ok(None)
    }
    async fn update_provider_config(
        &self,
        _id: uuid::Uuid,
        _priority: Option<i32>,
        _is_active: Option<bool>,
    ) -> Result<(), crate::db::DbError> {
        Ok(())
    }
    async fn disable_provider_by_name(&self, _provider: &str) -> Result<u64, crate::db::DbError> {
        Ok(0)
    }
    async fn get_message_by_id(
        &self,
        _id: uuid::Uuid,
    ) -> Result<Option<crate::db::Message>, crate::db::DbError> {
        Ok(None)
    }
    async fn search_messages(
        &self,
        _filters: &crate::routes::admin::messages::MessageSearchFilters,
    ) -> Result<Vec<crate::db::Message>, crate::db::DbError> {
        Ok(vec![])
    }
    async fn list_billing_accounts(
        &self,
    ) -> Result<Vec<crate::routes::admin::billing::BillingAccountSummary>, crate::db::DbError> {
        Ok(vec![])
    }
    async fn override_plan(
        &self,
        _account_id: uuid::Uuid,
        _plan_slug: &str,
    ) -> Result<(), crate::db::DbError> {
        Ok(())
    }
    async fn adjust_usage(
        &self,
        _account_id: uuid::Uuid,
        _sms_delta: Option<i32>,
        _email_delta: Option<i32>,
    ) -> Result<(), crate::db::DbError> {
        Ok(())
    }
    async fn billing_report(
        &self,
    ) -> Result<crate::routes::admin::billing::BillingReport, crate::db::DbError> {
        Ok(crate::routes::admin::billing::BillingReport {
            total_revenue_cents: 0,
            accounts_by_plan: vec![],
            overage_accounts: vec![],
        })
    }
    async fn list_all_webhooks(
        &self,
    ) -> Result<Vec<crate::routes::admin::webhooks::AdminWebhook>, crate::db::DbError> {
        Ok(vec![])
    }
    async fn get_webhook_by_id(
        &self,
        _id: uuid::Uuid,
    ) -> Result<Option<crate::db::Webhook>, crate::db::DbError> {
        Ok(None)
    }
    async fn get_webhook_deliveries(
        &self,
        _webhook_id: uuid::Uuid,
        _limit: i64,
        _offset: i64,
    ) -> Result<Vec<crate::routes::admin::webhooks::WebhookDelivery>, crate::db::DbError> {
        Ok(vec![])
    }
    async fn update_webhook_status(
        &self,
        _id: uuid::Uuid,
        _is_active: bool,
    ) -> Result<(), crate::db::DbError> {
        Ok(())
    }
    async fn disable_account_webhooks(
        &self,
        _account_id: uuid::Uuid,
    ) -> Result<u64, crate::db::DbError> {
        Ok(0)
    }
}
