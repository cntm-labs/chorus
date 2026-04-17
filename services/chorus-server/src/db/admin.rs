use async_trait::async_trait;
use sqlx::PgPool;
use uuid::Uuid;

use super::DbError;
use crate::routes::admin::accounts::{AccountDetail, AccountListItem};
use crate::routes::admin::providers::{AdminProviderConfig, ProviderHealth};

/// Repository for admin-only cross-account queries.
#[async_trait]
pub trait AdminRepository: Send + Sync {
    /// List all accounts.
    async fn list_accounts(&self) -> Result<Vec<AccountListItem>, DbError>;
    /// Get account detail with usage stats.
    async fn get_account_detail(&self, id: Uuid) -> Result<Option<AccountDetail>, DbError>;
    /// Create a new account.
    async fn create_account(&self, name: &str, email: &str) -> Result<AccountListItem, DbError>;
    /// Update account fields.
    async fn update_account(
        &self,
        id: Uuid,
        is_active: Option<bool>,
        name: Option<&str>,
    ) -> Result<(), DbError>;
    /// Deactivate (soft-delete) an account.
    async fn deactivate_account(&self, id: Uuid) -> Result<(), DbError>;

    // --- Provider Config (#35) ---

    /// List all provider configs across accounts.
    async fn list_all_provider_configs(&self) -> Result<Vec<AdminProviderConfig>, DbError>;
    /// Get provider health summary (error rate from recent delivery events).
    async fn get_provider_health(&self, id: Uuid) -> Result<Option<ProviderHealth>, DbError>;
    /// Update provider config (priority, is_active).
    async fn update_provider_config(
        &self,
        id: Uuid,
        priority: Option<i32>,
        is_active: Option<bool>,
    ) -> Result<(), DbError>;
    /// Disable a provider across all accounts (outage scenario).
    async fn disable_provider_by_name(&self, provider: &str) -> Result<u64, DbError>;
}

/// PostgreSQL implementation of admin repository.
pub struct PgAdminRepository {
    pool: PgPool,
}

impl PgAdminRepository {
    /// Create a new admin repository.
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl AdminRepository for PgAdminRepository {
    async fn list_accounts(&self) -> Result<Vec<AccountListItem>, DbError> {
        let rows = sqlx::query_as::<_, AccountListItem>(
            "SELECT id, name, owner_email, is_active, created_at
             FROM accounts ORDER BY created_at DESC",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| DbError::Internal(e.into()))?;

        Ok(rows)
    }

    async fn get_account_detail(&self, id: Uuid) -> Result<Option<AccountDetail>, DbError> {
        let row = sqlx::query_as::<_, AccountDetail>(
            "SELECT a.id, a.name, a.owner_email, a.is_active, a.created_at, a.updated_at,
                    (SELECT COUNT(*) FROM api_keys WHERE account_id = a.id) AS key_count,
                    (SELECT COUNT(*) FROM messages WHERE account_id = a.id) AS message_count
             FROM accounts a WHERE a.id = $1",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| DbError::Internal(e.into()))?;

        Ok(row)
    }

    async fn create_account(&self, name: &str, email: &str) -> Result<AccountListItem, DbError> {
        let row = sqlx::query_as::<_, AccountListItem>(
            "INSERT INTO accounts (name, owner_email) VALUES ($1, $2)
             RETURNING id, name, owner_email, is_active, created_at",
        )
        .bind(name)
        .bind(email)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| DbError::Internal(e.into()))?;

        Ok(row)
    }

    async fn update_account(
        &self,
        id: Uuid,
        is_active: Option<bool>,
        name: Option<&str>,
    ) -> Result<(), DbError> {
        sqlx::query(
            "UPDATE accounts SET
                is_active = COALESCE($1, is_active),
                name = COALESCE($2, name),
                updated_at = now()
             WHERE id = $3",
        )
        .bind(is_active)
        .bind(name)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(|e| DbError::Internal(e.into()))?;

        Ok(())
    }

    async fn deactivate_account(&self, id: Uuid) -> Result<(), DbError> {
        sqlx::query("UPDATE accounts SET is_active = false, updated_at = now() WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(|e| DbError::Internal(e.into()))?;

        Ok(())
    }

    async fn list_all_provider_configs(&self) -> Result<Vec<AdminProviderConfig>, DbError> {
        let rows = sqlx::query_as::<_, AdminProviderConfig>(
            "SELECT id, account_id, channel, provider, priority, is_active, created_at
             FROM provider_configs ORDER BY account_id, priority",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| DbError::Internal(e.into()))?;

        Ok(rows)
    }

    async fn get_provider_health(&self, id: Uuid) -> Result<Option<ProviderHealth>, DbError> {
        let row = sqlx::query_as::<_, ProviderHealth>(
            "SELECT pc.id, pc.provider,
                    COUNT(CASE WHEN m.status = 'delivered' THEN 1 END) AS total_sent,
                    COUNT(CASE WHEN m.status = 'failed' THEN 1 END) AS total_errors,
                    CASE
                        WHEN COUNT(*) FILTER (WHERE m.status IN ('delivered', 'failed')) = 0 THEN 0.0
                        ELSE COUNT(CASE WHEN m.status = 'failed' THEN 1 END)::float
                             / COUNT(*) FILTER (WHERE m.status IN ('delivered', 'failed'))
                    END AS error_rate,
                    MAX(CASE WHEN m.status = 'delivered' THEN m.delivered_at END) AS last_success,
                    MAX(CASE WHEN de.status = 'failed_attempt' THEN de.created_at END) AS last_error
             FROM provider_configs pc
             LEFT JOIN messages m ON m.provider = pc.provider AND m.account_id = pc.account_id
             LEFT JOIN delivery_events de ON de.message_id = m.id
             WHERE pc.id = $1
             GROUP BY pc.id, pc.provider",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| DbError::Internal(e.into()))?;

        Ok(row)
    }

    async fn update_provider_config(
        &self,
        id: Uuid,
        priority: Option<i32>,
        is_active: Option<bool>,
    ) -> Result<(), DbError> {
        sqlx::query(
            "UPDATE provider_configs SET
                priority = COALESCE($1, priority),
                is_active = COALESCE($2, is_active)
             WHERE id = $3",
        )
        .bind(priority)
        .bind(is_active)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(|e| DbError::Internal(e.into()))?;

        Ok(())
    }

    async fn disable_provider_by_name(&self, provider: &str) -> Result<u64, DbError> {
        let result =
            sqlx::query("UPDATE provider_configs SET is_active = false WHERE provider = $1")
                .bind(provider)
                .execute(&self.pool)
                .await
                .map_err(|e| DbError::Internal(e.into()))?;

        Ok(result.rows_affected())
    }
}
