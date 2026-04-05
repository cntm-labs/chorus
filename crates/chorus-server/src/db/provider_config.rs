use async_trait::async_trait;
use sqlx::PgPool;
use uuid::Uuid;

use super::{DbError, NewProviderConfig, ProviderConfig, ProviderConfigRepository};

/// PostgreSQL-backed provider config repository.
pub struct PgProviderConfigRepository {
    pool: PgPool,
}

impl PgProviderConfigRepository {
    /// Create a new repository backed by the given connection pool.
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl ProviderConfigRepository for PgProviderConfigRepository {
    async fn list_by_account_channel(
        &self,
        account_id: Uuid,
        channel: &str,
    ) -> Result<Vec<ProviderConfig>, DbError> {
        sqlx::query_as::<_, ProviderConfig>(
            "SELECT * FROM provider_configs
             WHERE account_id = $1 AND channel = $2 AND is_active = true
             ORDER BY priority ASC",
        )
        .bind(account_id)
        .bind(channel)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| DbError::Internal(e.into()))
    }

    async fn insert(&self, config: &NewProviderConfig) -> Result<ProviderConfig, DbError> {
        sqlx::query_as::<_, ProviderConfig>(
            "INSERT INTO provider_configs (account_id, channel, provider, priority, credentials)
             VALUES ($1, $2, $3, $4, $5)
             RETURNING *",
        )
        .bind(config.account_id)
        .bind(&config.channel)
        .bind(&config.provider)
        .bind(config.priority)
        .bind(&config.credentials)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| DbError::Internal(e.into()))
    }

    async fn list_by_account(&self, account_id: Uuid) -> Result<Vec<ProviderConfig>, DbError> {
        sqlx::query_as::<_, ProviderConfig>(
            "SELECT * FROM provider_configs WHERE account_id = $1 ORDER BY channel, priority ASC",
        )
        .bind(account_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| DbError::Internal(e.into()))
    }

    async fn delete(&self, id: Uuid, account_id: Uuid) -> Result<(), DbError> {
        let result = sqlx::query(
            "DELETE FROM provider_configs WHERE id = $1 AND account_id = $2",
        )
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
