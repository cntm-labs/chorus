use async_trait::async_trait;
use sqlx::PgPool;
use uuid::Uuid;

use super::{DbError, NewSuppression, Pagination, Suppression, SuppressionRepository};

/// PostgreSQL-backed suppression repository.
pub struct PgSuppressionRepository {
    pool: PgPool,
}

impl PgSuppressionRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl SuppressionRepository for PgSuppressionRepository {
    async fn is_suppressed(
        &self,
        account_id: Uuid,
        channel: &str,
        recipient: &str,
    ) -> Result<Option<String>, DbError> {
        let row: Option<(String,)> = sqlx::query_as(
            "SELECT reason FROM suppressions
             WHERE account_id = $1 AND channel = $2 AND recipient = $3",
        )
        .bind(account_id)
        .bind(channel)
        .bind(recipient)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| DbError::Internal(e.into()))?;

        Ok(row.map(|(r,)| r))
    }

    async fn add(&self, entry: &NewSuppression) -> Result<(), DbError> {
        sqlx::query(
            "INSERT INTO suppressions (account_id, channel, recipient, reason, source)
             VALUES ($1, $2, $3, $4, $5)
             ON CONFLICT (account_id, channel, recipient) DO NOTHING",
        )
        .bind(entry.account_id)
        .bind(&entry.channel)
        .bind(&entry.recipient)
        .bind(&entry.reason)
        .bind(&entry.source)
        .execute(&self.pool)
        .await
        .map_err(|e| DbError::Internal(e.into()))?;
        Ok(())
    }

    async fn remove(
        &self,
        account_id: Uuid,
        channel: &str,
        recipient: &str,
    ) -> Result<bool, DbError> {
        let result = sqlx::query(
            "DELETE FROM suppressions
             WHERE account_id = $1 AND channel = $2 AND recipient = $3",
        )
        .bind(account_id)
        .bind(channel)
        .bind(recipient)
        .execute(&self.pool)
        .await
        .map_err(|e| DbError::Internal(e.into()))?;
        Ok(result.rows_affected() > 0)
    }

    async fn list(
        &self,
        account_id: Uuid,
        channel: Option<&str>,
        pagination: &Pagination,
    ) -> Result<Vec<Suppression>, DbError> {
        let rows = if let Some(ch) = channel {
            sqlx::query_as::<_, Suppression>(
                "SELECT * FROM suppressions
                 WHERE account_id = $1 AND channel = $2
                 ORDER BY created_at DESC
                 LIMIT $3 OFFSET $4",
            )
            .bind(account_id)
            .bind(ch)
            .bind(pagination.limit)
            .bind(pagination.offset)
            .fetch_all(&self.pool)
            .await
        } else {
            sqlx::query_as::<_, Suppression>(
                "SELECT * FROM suppressions
                 WHERE account_id = $1
                 ORDER BY created_at DESC
                 LIMIT $2 OFFSET $3",
            )
            .bind(account_id)
            .bind(pagination.limit)
            .bind(pagination.offset)
            .fetch_all(&self.pool)
            .await
        };
        rows.map_err(|e| DbError::Internal(e.into()))
    }
}
