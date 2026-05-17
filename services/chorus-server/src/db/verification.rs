use async_trait::async_trait;
use sqlx::PgPool;
use uuid::Uuid;

use super::{DbError, NewVerification, Pagination, Verification, VerificationRepository};

/// PostgreSQL-backed verification repository.
pub struct PgVerificationRepository {
    pool: PgPool,
}

impl PgVerificationRepository {
    /// Create a new repository from a connection pool.
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

fn map_err(e: sqlx::Error) -> DbError {
    DbError::Internal(anyhow::Error::from(e))
}

#[async_trait]
impl VerificationRepository for PgVerificationRepository {
    async fn insert(&self, v: &NewVerification) -> Result<Verification, DbError> {
        let row: Verification = sqlx::query_as(
            "INSERT INTO verifications
                (account_id, api_key_id, channel, recipient, status,
                 cost_micro, environment, app_name, expires_at)
             VALUES ($1, $2, $3, $4, 'pending',
                     $5, $6, $7, now() + interval '5 minutes')
             RETURNING *",
        )
        .bind(v.account_id)
        .bind(v.api_key_id)
        .bind(&v.channel)
        .bind(&v.recipient)
        .bind(v.initial_cost_micro)
        .bind(&v.environment)
        .bind(v.app_name.as_deref())
        .fetch_one(&self.pool)
        .await
        .map_err(map_err)?;
        Ok(row)
    }

    async fn find_by_id(
        &self,
        id: Uuid,
        account_id: Uuid,
    ) -> Result<Option<Verification>, DbError> {
        let row: Option<Verification> =
            sqlx::query_as("SELECT * FROM verifications WHERE id = $1 AND account_id = $2")
                .bind(id)
                .bind(account_id)
                .fetch_optional(&self.pool)
                .await
                .map_err(map_err)?;
        Ok(row)
    }

    async fn list_by_account(
        &self,
        account_id: Uuid,
        pagination: &Pagination,
    ) -> Result<Vec<Verification>, DbError> {
        let rows: Vec<Verification> = sqlx::query_as(
            "SELECT * FROM verifications
             WHERE account_id = $1
             ORDER BY created_at DESC
             LIMIT $2 OFFSET $3",
        )
        .bind(account_id)
        .bind(pagination.limit)
        .bind(pagination.offset)
        .fetch_all(&self.pool)
        .await
        .map_err(map_err)?;
        Ok(rows)
    }

    async fn increment_check_attempts(&self, id: Uuid, account_id: Uuid) -> Result<i32, DbError> {
        let row: Option<(i32,)> = sqlx::query_as(
            "UPDATE verifications
             SET check_attempts = check_attempts + 1, updated_at = now()
             WHERE id = $1 AND account_id = $2 AND status = 'pending'
             RETURNING check_attempts",
        )
        .bind(id)
        .bind(account_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(map_err)?;
        row.map(|(n,)| n).ok_or(DbError::NotFound)
    }

    async fn mark_approved(&self, id: Uuid, account_id: Uuid) -> Result<(), DbError> {
        let result = sqlx::query(
            "UPDATE verifications
             SET status = 'approved', updated_at = now()
             WHERE id = $1 AND account_id = $2 AND status = 'pending'",
        )
        .bind(id)
        .bind(account_id)
        .execute(&self.pool)
        .await
        .map_err(map_err)?;
        if result.rows_affected() == 0 {
            return Err(DbError::NotFound);
        }
        Ok(())
    }

    async fn mark_canceled(&self, id: Uuid, account_id: Uuid) -> Result<bool, DbError> {
        let result = sqlx::query(
            "UPDATE verifications
             SET status = 'canceled', updated_at = now()
             WHERE id = $1 AND account_id = $2 AND status = 'pending'",
        )
        .bind(id)
        .bind(account_id)
        .execute(&self.pool)
        .await
        .map_err(map_err)?;
        Ok(result.rows_affected() > 0)
    }

    async fn record_resend(
        &self,
        id: Uuid,
        account_id: Uuid,
        additional_cost_micro: i64,
        max_resends: i32,
    ) -> Result<Verification, DbError> {
        let row: Option<Verification> = sqlx::query_as(
            "UPDATE verifications
             SET resend_attempts = resend_attempts + 1,
                 cost_micro      = cost_micro + $3,
                 check_attempts  = 0,
                 updated_at      = now()
             WHERE id = $1 AND account_id = $2
               AND status = 'pending'
               AND resend_attempts < $4
             RETURNING *",
        )
        .bind(id)
        .bind(account_id)
        .bind(additional_cost_micro)
        .bind(max_resends)
        .fetch_optional(&self.pool)
        .await
        .map_err(map_err)?;
        row.ok_or(DbError::NotFound)
    }

    async fn expire_pending(&self, limit: i64) -> Result<u64, DbError> {
        let result = sqlx::query(
            "UPDATE verifications
             SET status = 'expired', updated_at = now()
             WHERE id IN (
                 SELECT id FROM verifications
                 WHERE status = 'pending' AND expires_at < now()
                 ORDER BY expires_at
                 LIMIT $1
             )",
        )
        .bind(limit)
        .execute(&self.pool)
        .await
        .map_err(map_err)?;
        Ok(result.rows_affected())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{NewVerification, Pagination, VerificationRepository};

    async fn seed_api_key(pool: &PgPool) -> (Uuid, Uuid) {
        let account_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO accounts (id, name, owner_email, is_active)
             VALUES ($1, 'test', 'test@example.com', true)",
        )
        .bind(account_id)
        .execute(pool)
        .await
        .unwrap();
        let key_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO api_keys (id, account_id, name, key_hash, key_prefix, environment)
             VALUES ($1, $2, 'k', $3, 'ch_test_xx', 'test')",
        )
        .bind(key_id)
        .bind(account_id)
        .bind(format!("hash-{key_id}"))
        .execute(pool)
        .await
        .unwrap();
        (account_id, key_id)
    }

    fn fixture(account_id: Uuid, key_id: Uuid, channel: &str, recipient: &str) -> NewVerification {
        NewVerification {
            account_id,
            api_key_id: key_id,
            channel: channel.to_string(),
            recipient: recipient.to_string(),
            environment: "test".to_string(),
            app_name: Some("Acme".to_string()),
            initial_cost_micro: 100,
        }
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./src/db/migrations")]
    async fn insert_creates_pending_row(pool: PgPool) {
        let (acct, key) = seed_api_key(&pool).await;
        let repo = PgVerificationRepository::new(pool);
        let v = repo
            .insert(&fixture(acct, key, "email", "alice@example.com"))
            .await
            .unwrap();
        assert_eq!(v.status, "pending");
        assert_eq!(v.channel, "email");
        assert_eq!(v.cost_micro, 100);
        assert_eq!(v.check_attempts, 0);
        assert_eq!(v.resend_attempts, 0);
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./src/db/migrations")]
    async fn find_by_id_scopes_to_account(pool: PgPool) {
        let (acct, key) = seed_api_key(&pool).await;
        let repo = PgVerificationRepository::new(pool);
        let v = repo
            .insert(&fixture(acct, key, "email", "alice@example.com"))
            .await
            .unwrap();
        let other_acct = Uuid::new_v4();
        assert!(repo.find_by_id(v.id, other_acct).await.unwrap().is_none());
        assert!(repo.find_by_id(v.id, acct).await.unwrap().is_some());
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./src/db/migrations")]
    async fn list_by_account_orders_desc(pool: PgPool) {
        let (acct, key) = seed_api_key(&pool).await;
        let repo = PgVerificationRepository::new(pool);
        for i in 0..3 {
            repo.insert(&fixture(acct, key, "email", &format!("u{i}@example.com")))
                .await
                .unwrap();
        }
        let rows = repo
            .list_by_account(
                acct,
                &Pagination {
                    limit: 10,
                    offset: 0,
                },
            )
            .await
            .unwrap();
        assert_eq!(rows.len(), 3);
        assert!(rows[0].created_at >= rows[1].created_at);
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./src/db/migrations")]
    async fn increment_check_attempts_returns_new_count(pool: PgPool) {
        let (acct, key) = seed_api_key(&pool).await;
        let repo = PgVerificationRepository::new(pool);
        let v = repo
            .insert(&fixture(acct, key, "email", "alice@example.com"))
            .await
            .unwrap();
        assert_eq!(repo.increment_check_attempts(v.id, acct).await.unwrap(), 1);
        assert_eq!(repo.increment_check_attempts(v.id, acct).await.unwrap(), 2);
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./src/db/migrations")]
    async fn increment_check_attempts_errors_when_terminal(pool: PgPool) {
        let (acct, key) = seed_api_key(&pool).await;
        let repo = PgVerificationRepository::new(pool);
        let v = repo
            .insert(&fixture(acct, key, "email", "alice@example.com"))
            .await
            .unwrap();
        repo.mark_approved(v.id, acct).await.unwrap();
        let err = repo.increment_check_attempts(v.id, acct).await.unwrap_err();
        assert!(matches!(err, DbError::NotFound));
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./src/db/migrations")]
    async fn mark_canceled_only_when_pending(pool: PgPool) {
        let (acct, key) = seed_api_key(&pool).await;
        let repo = PgVerificationRepository::new(pool);
        let v = repo
            .insert(&fixture(acct, key, "email", "a@b.com"))
            .await
            .unwrap();
        assert!(repo.mark_canceled(v.id, acct).await.unwrap());
        assert!(!repo.mark_canceled(v.id, acct).await.unwrap());
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./src/db/migrations")]
    async fn record_resend_increments_and_adds_cost(pool: PgPool) {
        let (acct, key) = seed_api_key(&pool).await;
        let repo = PgVerificationRepository::new(pool);
        let v = repo
            .insert(&fixture(acct, key, "email", "a@b.com"))
            .await
            .unwrap();
        let updated = repo.record_resend(v.id, acct, 6000, 3).await.unwrap();
        assert_eq!(updated.resend_attempts, 1);
        assert_eq!(updated.cost_micro, 6100);
        assert_eq!(updated.check_attempts, 0);
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./src/db/migrations")]
    async fn record_resend_returns_notfound_when_max_reached(pool: PgPool) {
        let (acct, key) = seed_api_key(&pool).await;
        let repo = PgVerificationRepository::new(pool);
        let v = repo
            .insert(&fixture(acct, key, "email", "a@b.com"))
            .await
            .unwrap();
        for _ in 0..3 {
            repo.record_resend(v.id, acct, 100, 3).await.unwrap();
        }
        let err = repo.record_resend(v.id, acct, 100, 3).await.unwrap_err();
        assert!(matches!(err, DbError::NotFound));
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./src/db/migrations")]
    async fn expire_pending_only_picks_expired_pending(pool: PgPool) {
        let (acct, key) = seed_api_key(&pool).await;
        let repo = PgVerificationRepository::new(pool.clone());
        for i in 0..3 {
            let v = repo
                .insert(&fixture(acct, key, "email", &format!("u{i}@example.com")))
                .await
                .unwrap();
            sqlx::query(
                "UPDATE verifications SET expires_at = now() - interval '1s' WHERE id = $1",
            )
            .bind(v.id)
            .execute(&pool)
            .await
            .unwrap();
        }
        // One non-expired
        repo.insert(&fixture(acct, key, "email", "fresh@example.com"))
            .await
            .unwrap();

        let count = repo.expire_pending(100).await.unwrap();
        assert_eq!(count, 3);

        let still_pending: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM verifications WHERE status='pending' AND account_id=$1",
        )
        .bind(acct)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(still_pending, 1);
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./src/db/migrations")]
    async fn cascade_delete_on_api_key_removal(pool: PgPool) {
        let (acct, key) = seed_api_key(&pool).await;
        let repo = PgVerificationRepository::new(pool.clone());
        repo.insert(&fixture(acct, key, "email", "a@b.com"))
            .await
            .unwrap();
        sqlx::query("DELETE FROM api_keys WHERE id = $1")
            .bind(key)
            .execute(&pool)
            .await
            .unwrap();
        let n: i64 = sqlx::query_scalar("SELECT count(*) FROM verifications WHERE account_id = $1")
            .bind(acct)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(n, 0);
    }
}
