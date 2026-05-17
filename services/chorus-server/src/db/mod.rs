pub mod admin;
pub mod billing;
pub mod idempotency;
pub mod postgres;
pub mod provider_config;
pub mod suppression;
pub mod verification;
pub mod webhook;

pub use admin::{AdminRepository, PgAdminRepository};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Database error types for the repository layer.
#[derive(Debug, thiserror::Error)]
pub enum DbError {
    /// Requested entity was not found.
    #[error("not found")]
    NotFound,
    /// Statement timed out — used to signal a busy idempotency lock.
    #[error("statement timeout")]
    Timeout,
    /// Internal database error.
    #[error("database error: {0}")]
    Internal(#[from] anyhow::Error),
}

/// An account in the system.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Account {
    pub id: Uuid,
    pub name: String,
    pub owner_email: String,
    pub is_active: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// An API key belonging to an account.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct ApiKey {
    pub id: Uuid,
    pub account_id: Uuid,
    pub name: String,
    pub key_prefix: String,
    pub environment: String,
    pub last_used_at: Option<DateTime<Utc>>,
    pub expires_at: Option<DateTime<Utc>>,
    pub is_revoked: bool,
    pub created_at: DateTime<Utc>,
}

/// A message record with delivery status.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Message {
    pub id: Uuid,
    pub account_id: Uuid,
    pub api_key_id: Uuid,
    pub channel: String,
    pub provider: Option<String>,
    pub sender: Option<String>,
    pub recipient: String,
    pub subject: Option<String>,
    pub body: String,
    pub status: String,
    pub provider_message_id: Option<String>,
    pub error_message: Option<String>,
    pub cost_microdollars: i64,
    pub attempts: i32,
    pub environment: String,
    pub created_at: DateTime<Utc>,
    pub delivered_at: Option<DateTime<Utc>>,
}

/// A delivery event for audit trail.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct DeliveryEvent {
    pub id: Uuid,
    pub message_id: Uuid,
    pub status: String,
    pub provider_data: Option<serde_json::Value>,
    pub created_at: DateTime<Utc>,
}

/// Parameters for inserting a new message.
pub struct NewMessage {
    pub account_id: Uuid,
    pub api_key_id: Uuid,
    pub channel: String,
    pub sender: Option<String>,
    pub recipient: String,
    pub subject: Option<String>,
    pub body: String,
    pub environment: String,
}

/// Pagination parameters for list queries.
pub struct Pagination {
    pub limit: i64,
    pub offset: i64,
}

/// Account lookup and key usage tracking.
#[async_trait]
pub trait AccountRepository: Send + Sync {
    /// Find an account and its API key by the key's SHA-256 hash.
    async fn find_by_api_key_hash(&self, hash: &str) -> Result<Option<(Account, ApiKey)>, DbError>;

    /// Update the `last_used_at` timestamp for an API key.
    async fn update_key_last_used(&self, key_id: Uuid) -> Result<(), DbError>;
}

/// Message CRUD and delivery event tracking.
#[async_trait]
pub trait MessageRepository: Send + Sync {
    /// Insert a new message in `queued` status.
    async fn insert(&self, msg: &NewMessage) -> Result<Message, DbError>;

    /// Find a message by ID scoped to an account.
    async fn find_by_id(&self, id: Uuid, account_id: Uuid) -> Result<Option<Message>, DbError>;

    /// List messages for an account with pagination.
    async fn list_by_account(
        &self,
        account_id: Uuid,
        pagination: &Pagination,
    ) -> Result<Vec<Message>, DbError>;

    /// Update message delivery status and provider info.
    async fn update_status(
        &self,
        id: Uuid,
        status: &str,
        provider: Option<&str>,
        provider_message_id: Option<&str>,
        error_message: Option<&str>,
    ) -> Result<(), DbError>;

    /// Record a delivery event for audit trail.
    async fn insert_delivery_event(
        &self,
        message_id: Uuid,
        status: &str,
        provider_data: Option<serde_json::Value>,
    ) -> Result<(), DbError>;

    /// Get all delivery events for a message.
    async fn get_delivery_events(&self, message_id: Uuid) -> Result<Vec<DeliveryEvent>, DbError>;

    /// Find a message by its provider's message id (no account scoping — internal use only).
    async fn find_by_provider_message_id(
        &self,
        provider_message_id: &str,
    ) -> Result<Option<Message>, DbError>;
}

/// A provider configuration for per-account routing.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct ProviderConfig {
    pub id: Uuid,
    pub account_id: Uuid,
    pub channel: String,
    pub provider: String,
    pub priority: i32,
    pub credentials: serde_json::Value,
    pub is_active: bool,
    pub created_at: DateTime<Utc>,
}

/// Parameters for inserting a new provider config.
pub struct NewProviderConfig {
    pub account_id: Uuid,
    pub channel: String,
    pub provider: String,
    pub priority: i32,
    pub credentials: serde_json::Value,
}

/// Per-account provider configuration management.
#[async_trait]
pub trait ProviderConfigRepository: Send + Sync {
    /// List active provider configs for an account+channel, ordered by priority.
    async fn list_by_account_channel(
        &self,
        account_id: Uuid,
        channel: &str,
    ) -> Result<Vec<ProviderConfig>, DbError>;

    /// Insert a new provider config.
    async fn insert(&self, config: &NewProviderConfig) -> Result<ProviderConfig, DbError>;

    /// List all provider configs for an account.
    async fn list_by_account(&self, account_id: Uuid) -> Result<Vec<ProviderConfig>, DbError>;

    /// Delete a provider config.
    async fn delete(&self, id: Uuid, account_id: Uuid) -> Result<(), DbError>;
}

/// A webhook registration for delivery callbacks.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Webhook {
    pub id: Uuid,
    pub account_id: Uuid,
    pub url: String,
    pub secret: String,
    pub events: Vec<String>,
    pub is_active: bool,
    pub created_at: DateTime<Utc>,
}

/// Parameters for registering a new webhook.
pub struct NewWebhook {
    pub account_id: Uuid,
    pub url: String,
    pub secret: String,
    pub events: Vec<String>,
}

/// Webhook registration management.
#[async_trait]
pub trait WebhookRepository: Send + Sync {
    /// Insert a new webhook.
    async fn insert(&self, webhook: &NewWebhook) -> Result<Webhook, DbError>;

    /// List all active webhooks for an account.
    async fn list_by_account(&self, account_id: Uuid) -> Result<Vec<Webhook>, DbError>;

    /// List webhooks matching an account and event type.
    async fn list_by_account_event(
        &self,
        account_id: Uuid,
        event: &str,
    ) -> Result<Vec<Webhook>, DbError>;

    /// Delete a webhook (soft-delete by setting `is_active = false`).
    async fn delete(&self, id: Uuid, account_id: Uuid) -> Result<(), DbError>;
}

/// An admin API key for dashboard access.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct AdminKey {
    pub id: Uuid,
    pub name: String,
    pub key_prefix: String,
    pub is_revoked: bool,
    pub created_at: DateTime<Utc>,
}

/// Repository for admin key operations.
#[async_trait]
pub trait AdminKeyRepository: Send + Sync {
    /// Find an admin key by its SHA-256 hash.
    async fn find_by_hash(&self, hash: &str) -> Result<Option<AdminKey>, DbError>;
}

/// API key management operations.
#[async_trait]
pub trait ApiKeyRepository: Send + Sync {
    /// List all API keys for an account.
    async fn list_by_account(&self, account_id: Uuid) -> Result<Vec<ApiKey>, DbError>;

    /// Create a new API key.
    async fn insert(
        &self,
        account_id: Uuid,
        name: &str,
        key_hash: &str,
        key_prefix: &str,
        environment: &str,
    ) -> Result<ApiKey, DbError>;

    /// Soft-revoke an API key.
    async fn revoke(&self, id: Uuid, account_id: Uuid) -> Result<(), DbError>;
}

/// A suppression list entry for a recipient that should not receive messages.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Suppression {
    pub account_id: Uuid,
    pub channel: String,
    pub recipient: String,
    pub reason: String,
    pub source: String,
    pub created_at: DateTime<Utc>,
}

/// Parameters for inserting a new suppression entry.
pub struct NewSuppression {
    pub account_id: Uuid,
    pub channel: String,
    pub recipient: String,
    pub reason: String,
    pub source: String,
}

/// Outcome of a suppression `add` call.
pub struct AddSuppressionResult {
    /// The canonical row (newly inserted or pre-existing).
    pub entry: Suppression,
    /// `true` if this call inserted the row; `false` if it already existed.
    pub inserted: bool,
}

/// Suppression list management.
#[async_trait]
pub trait SuppressionRepository: Send + Sync {
    /// Returns the suppression `reason` if `recipient` is suppressed for the given account+channel.
    async fn is_suppressed(
        &self,
        account_id: Uuid,
        channel: &str,
        recipient: &str,
    ) -> Result<Option<String>, DbError>;

    /// Insert a suppression idempotently. Returns the canonical row plus a flag
    /// indicating whether this call performed the insert.
    async fn add(&self, entry: &NewSuppression) -> Result<AddSuppressionResult, DbError>;

    /// Remove a suppression. Returns `true` if a row was deleted.
    async fn remove(
        &self,
        account_id: Uuid,
        channel: &str,
        recipient: &str,
    ) -> Result<bool, DbError>;

    /// List suppressions for an account, optionally filtered by channel, with pagination.
    async fn list(
        &self,
        account_id: Uuid,
        channel: Option<&str>,
        pagination: &Pagination,
    ) -> Result<Vec<Suppression>, DbError>;
}

/// An idempotency record for a previously-seen request.
#[derive(Debug, Clone)]
pub struct IdempotencyRecord {
    pub api_key_id: Uuid,
    pub idempotency_key: String,
    pub request_hash: [u8; 32],
    pub request_method: String,
    pub request_path: String,
    pub status: IdempotencyStatus,
    pub response_status: Option<u16>,
    pub response_body: Option<Vec<u8>>,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

/// Lifecycle status for an idempotency record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IdempotencyStatus {
    /// Record was inserted but the original request has not yet completed.
    InProgress,
    /// Record has a cached response and can be replayed.
    Completed,
}

/// Outcome of an `IdempotencyRepository::begin` call.
#[derive(Debug, Clone)]
pub enum IdempotencyOutcome {
    /// First time this key has been seen — caller proceeds and calls `complete`.
    Fresh,
    /// Existing completed row with matching hash — caller returns this response verbatim.
    Replay { status: u16, body: Vec<u8> },
    /// Existing row with a different request hash — caller returns 422.
    HashMismatch,
}

/// Idempotency record management.
#[async_trait]
pub trait IdempotencyRepository: Send + Sync {
    /// Atomically insert a fresh `in_progress` row, or read an existing row under
    /// a row-level lock. Stale `in_progress` rows older than 60 s are recovered.
    async fn begin(
        &self,
        api_key_id: Uuid,
        key: &str,
        request_hash: &[u8; 32],
        method: &str,
        path: &str,
    ) -> Result<IdempotencyOutcome, DbError>;

    /// Mark an `in_progress` row as `completed` and store the response.
    async fn complete(
        &self,
        api_key_id: Uuid,
        key: &str,
        response_status: u16,
        response_body: &[u8],
    ) -> Result<(), DbError>;

    /// Delete up to `limit` rows where `expires_at < now()`.
    /// Returns the number of rows actually deleted.
    async fn delete_expired(&self, limit: i64) -> Result<u64, DbError>;
}

/// A verification (OTP) lifecycle record.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Verification {
    pub id: Uuid,
    pub account_id: Uuid,
    pub api_key_id: Uuid,
    pub channel: String,
    pub recipient: String,
    pub status: String,
    pub check_attempts: i32,
    pub resend_attempts: i32,
    pub cost_micro: i64,
    pub cost_currency: String,
    pub environment: String,
    pub app_name: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

/// Parameters for inserting a new verification.
pub struct NewVerification {
    pub account_id: Uuid,
    pub api_key_id: Uuid,
    pub channel: String,
    pub recipient: String,
    pub environment: String,
    pub app_name: Option<String>,
    pub initial_cost_micro: i64,
}

/// Verification lifecycle and counters.
#[async_trait]
pub trait VerificationRepository: Send + Sync {
    /// Insert a new pending verification (expires_at = now() + 5 min).
    async fn insert(&self, v: &NewVerification) -> Result<Verification, DbError>;

    /// Find by id scoped to an account.
    async fn find_by_id(&self, id: Uuid, account_id: Uuid)
        -> Result<Option<Verification>, DbError>;

    /// List for an account ordered by created_at DESC.
    async fn list_by_account(
        &self,
        account_id: Uuid,
        pagination: &Pagination,
    ) -> Result<Vec<Verification>, DbError>;

    /// Increment `check_attempts` atomically; returns the new count.
    /// Errors with `NotFound` if status != 'pending'.
    async fn increment_check_attempts(&self, id: Uuid, account_id: Uuid) -> Result<i32, DbError>;

    /// Set status='approved' (only if currently pending). Returns NotFound otherwise.
    async fn mark_approved(&self, id: Uuid, account_id: Uuid) -> Result<(), DbError>;

    /// Set status='canceled' only if currently pending. Returns true on success.
    async fn mark_canceled(&self, id: Uuid, account_id: Uuid) -> Result<bool, DbError>;

    /// Atomic resend: increments resend_attempts, adds cost, resets check_attempts.
    /// Errors with NotFound if not pending or resend cap reached.
    async fn record_resend(
        &self,
        id: Uuid,
        account_id: Uuid,
        additional_cost_micro: i64,
        max_resends: i32,
    ) -> Result<Verification, DbError>;

    /// Cleanup: bulk-mark expired pending rows. Returns count.
    async fn expire_pending(&self, limit: i64) -> Result<u64, DbError>;
}
