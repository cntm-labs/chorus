pub mod postgres;

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
    async fn find_by_api_key_hash(
        &self,
        hash: &str,
    ) -> Result<Option<(Account, ApiKey)>, DbError>;

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
