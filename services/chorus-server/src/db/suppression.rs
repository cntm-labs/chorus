use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use super::{
    AddSuppressionResult, DbError, NewSuppression, Pagination, Suppression, SuppressionRepository,
};

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

    async fn add(&self, entry: &NewSuppression) -> Result<AddSuppressionResult, DbError> {
        // Upsert with no-op conflict update so RETURNING * always yields a row.
        // `xmax = 0` is a Postgres idiom: zero on a freshly inserted row,
        // non-zero on an updated/conflicted row — distinguishes insert vs hit.
        let row: (Uuid, String, String, String, String, DateTime<Utc>, bool) = sqlx::query_as(
            "INSERT INTO suppressions (account_id, channel, recipient, reason, source)
             VALUES ($1, $2, $3, $4, $5)
             ON CONFLICT (account_id, channel, recipient)
             DO UPDATE SET account_id = EXCLUDED.account_id
             RETURNING account_id, channel, recipient, reason, source, created_at, (xmax = 0) AS inserted",
        )
        .bind(entry.account_id)
        .bind(&entry.channel)
        .bind(&entry.recipient)
        .bind(&entry.reason)
        .bind(&entry.source)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| DbError::Internal(e.into()))?;

        Ok(AddSuppressionResult {
            entry: Suppression {
                account_id: row.0,
                channel: row.1,
                recipient: row.2,
                reason: row.3,
                source: row.4,
                created_at: row.5,
            },
            inserted: row.6,
        })
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
