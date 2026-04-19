use async_trait::async_trait;
use sqlx::PgPool;
use std::time::Instant;
use uuid::Uuid;

/// Record DB query duration for a given operation.
macro_rules! record_db_duration {
    ($op:expr, $start:expr) => {
        metrics::histogram!(
            "chorus_db_query_duration_seconds",
            "operation" => $op,
        )
        .record($start.elapsed().as_secs_f64());
    };
}

use super::{
    Account, AccountRepository, AdminKey, AdminKeyRepository, ApiKey, ApiKeyRepository, DbError,
    DeliveryEvent, Message, MessageRepository, NewMessage, Pagination,
};

/// PostgreSQL-backed repository implementation using sqlx.
pub struct PgRepository {
    pool: PgPool,
}

impl PgRepository {
    /// Create a new repository backed by the given connection pool.
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl AccountRepository for PgRepository {
    async fn find_by_api_key_hash(&self, hash: &str) -> Result<Option<(Account, ApiKey)>, DbError> {
        let row = sqlx::query_as::<_, Account>(
            "SELECT a.id, a.name, a.owner_email, a.is_active, a.created_at, a.updated_at
             FROM accounts a
             INNER JOIN api_keys k ON k.account_id = a.id
             WHERE k.key_hash = $1",
        )
        .bind(hash)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| DbError::Internal(e.into()))?;

        let Some(account) = row else {
            return Ok(None);
        };

        let key = sqlx::query_as::<_, ApiKey>(
            "SELECT id, account_id, name, key_prefix, environment,
                    last_used_at, expires_at, is_revoked, created_at
             FROM api_keys WHERE key_hash = $1",
        )
        .bind(hash)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| DbError::Internal(e.into()))?;

        Ok(Some((account, key)))
    }

    async fn update_key_last_used(&self, key_id: Uuid) -> Result<(), DbError> {
        sqlx::query("UPDATE api_keys SET last_used_at = now() WHERE id = $1")
            .bind(key_id)
            .execute(&self.pool)
            .await
            .map_err(|e| DbError::Internal(e.into()))?;
        Ok(())
    }
}

#[async_trait]
impl MessageRepository for PgRepository {
    async fn insert(&self, msg: &NewMessage) -> Result<Message, DbError> {
        let start = Instant::now();
        let message = sqlx::query_as::<_, Message>(
            "INSERT INTO messages (account_id, api_key_id, channel, sender, recipient, subject, body, environment)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
             RETURNING *",
        )
        .bind(msg.account_id)
        .bind(msg.api_key_id)
        .bind(&msg.channel)
        .bind(&msg.sender)
        .bind(&msg.recipient)
        .bind(&msg.subject)
        .bind(&msg.body)
        .bind(&msg.environment)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| DbError::Internal(e.into()))?;
        record_db_duration!("insert_message", start);

        Ok(message)
    }

    async fn find_by_id(&self, id: Uuid, account_id: Uuid) -> Result<Option<Message>, DbError> {
        let start = Instant::now();
        let msg = sqlx::query_as::<_, Message>(
            "SELECT * FROM messages WHERE id = $1 AND account_id = $2",
        )
        .bind(id)
        .bind(account_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| DbError::Internal(e.into()))?;
        record_db_duration!("find_by_id", start);

        Ok(msg)
    }

    async fn list_by_account(
        &self,
        account_id: Uuid,
        pagination: &Pagination,
    ) -> Result<Vec<Message>, DbError> {
        let messages = sqlx::query_as::<_, Message>(
            "SELECT * FROM messages WHERE account_id = $1
             ORDER BY created_at DESC
             LIMIT $2 OFFSET $3",
        )
        .bind(account_id)
        .bind(pagination.limit)
        .bind(pagination.offset)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| DbError::Internal(e.into()))?;

        Ok(messages)
    }

    async fn update_status(
        &self,
        id: Uuid,
        status: &str,
        provider: Option<&str>,
        provider_message_id: Option<&str>,
        error_message: Option<&str>,
    ) -> Result<(), DbError> {
        let start = Instant::now();
        sqlx::query(
            "UPDATE messages SET status = $1, provider = $2, provider_message_id = $3,
             error_message = $4, attempts = attempts + 1,
             delivered_at = CASE WHEN $1 = 'delivered' THEN now() ELSE delivered_at END
             WHERE id = $5",
        )
        .bind(status)
        .bind(provider)
        .bind(provider_message_id)
        .bind(error_message)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(|e| DbError::Internal(e.into()))?;
        record_db_duration!("update_status", start);

        Ok(())
    }

    async fn insert_delivery_event(
        &self,
        message_id: Uuid,
        status: &str,
        provider_data: Option<serde_json::Value>,
    ) -> Result<(), DbError> {
        let start = Instant::now();
        sqlx::query(
            "INSERT INTO delivery_events (message_id, status, provider_data)
             VALUES ($1, $2, $3)",
        )
        .bind(message_id)
        .bind(status)
        .bind(provider_data)
        .execute(&self.pool)
        .await
        .map_err(|e| DbError::Internal(e.into()))?;
        record_db_duration!("insert_delivery_event", start);

        Ok(())
    }

    async fn get_delivery_events(&self, message_id: Uuid) -> Result<Vec<DeliveryEvent>, DbError> {
        let events = sqlx::query_as::<_, DeliveryEvent>(
            "SELECT * FROM delivery_events WHERE message_id = $1 ORDER BY created_at",
        )
        .bind(message_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| DbError::Internal(e.into()))?;

        Ok(events)
    }
}

#[async_trait]
impl ApiKeyRepository for PgRepository {
    async fn list_by_account(&self, account_id: Uuid) -> Result<Vec<ApiKey>, DbError> {
        let keys = sqlx::query_as::<_, ApiKey>(
            "SELECT id, account_id, name, key_prefix, environment,
                    last_used_at, expires_at, is_revoked, created_at
             FROM api_keys WHERE account_id = $1
             ORDER BY created_at DESC",
        )
        .bind(account_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| DbError::Internal(e.into()))?;

        Ok(keys)
    }

    async fn insert(
        &self,
        account_id: Uuid,
        name: &str,
        key_hash: &str,
        key_prefix: &str,
        environment: &str,
    ) -> Result<ApiKey, DbError> {
        let key = sqlx::query_as::<_, ApiKey>(
            "INSERT INTO api_keys (account_id, name, key_hash, key_prefix, environment)
             VALUES ($1, $2, $3, $4, $5)
             RETURNING id, account_id, name, key_prefix, environment,
                       last_used_at, expires_at, is_revoked, created_at",
        )
        .bind(account_id)
        .bind(name)
        .bind(key_hash)
        .bind(key_prefix)
        .bind(environment)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| DbError::Internal(e.into()))?;

        Ok(key)
    }

    async fn revoke(&self, id: Uuid, account_id: Uuid) -> Result<(), DbError> {
        let result =
            sqlx::query("UPDATE api_keys SET is_revoked = true WHERE id = $1 AND account_id = $2")
                .bind(id)
                .bind(account_id)
                .execute(&self.pool)
                .await
                .map_err(|e| DbError::Internal(e.into()))?;

        if result.rows_affected() == 0 {
            return Err(DbError::NotFound);
        }

        Ok(())
    }
}

#[async_trait]
impl AdminKeyRepository for PgRepository {
    async fn find_by_hash(&self, hash: &str) -> Result<Option<AdminKey>, DbError> {
        let key = sqlx::query_as::<_, AdminKey>(
            "SELECT id, name, key_prefix, is_revoked, created_at
             FROM admin_keys WHERE key_hash = $1",
        )
        .bind(hash)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| DbError::Internal(e.into()))?;

        Ok(key)
    }
}
