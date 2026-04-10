use async_trait::async_trait;
use sqlx::PgPool;
use uuid::Uuid;

use super::{DbError, NewWebhook, Webhook, WebhookRepository};

/// PostgreSQL-backed webhook repository.
pub struct PgWebhookRepository {
    pool: PgPool,
}

impl PgWebhookRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl WebhookRepository for PgWebhookRepository {
    async fn insert(&self, webhook: &NewWebhook) -> Result<Webhook, DbError> {
        let row = sqlx::query_as::<_, Webhook>(
            r#"INSERT INTO webhooks (account_id, url, secret, events)
               VALUES ($1, $2, $3, $4)
               RETURNING *"#,
        )
        .bind(webhook.account_id)
        .bind(&webhook.url)
        .bind(&webhook.secret)
        .bind(&webhook.events)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| DbError::Internal(e.into()))?;
        Ok(row)
    }

    async fn list_by_account(&self, account_id: Uuid) -> Result<Vec<Webhook>, DbError> {
        let rows = sqlx::query_as::<_, Webhook>(
            "SELECT * FROM webhooks WHERE account_id = $1 AND is_active = true ORDER BY created_at",
        )
        .bind(account_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| DbError::Internal(e.into()))?;
        Ok(rows)
    }

    async fn list_by_account_event(
        &self,
        account_id: Uuid,
        event: &str,
    ) -> Result<Vec<Webhook>, DbError> {
        let rows = sqlx::query_as::<_, Webhook>(
            "SELECT * FROM webhooks WHERE account_id = $1 AND is_active = true AND $2 = ANY(events)",
        )
        .bind(account_id)
        .bind(event)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| DbError::Internal(e.into()))?;
        Ok(rows)
    }

    async fn delete(&self, id: Uuid, account_id: Uuid) -> Result<(), DbError> {
        let result = sqlx::query(
            "UPDATE webhooks SET is_active = false WHERE id = $1 AND account_id = $2",
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
