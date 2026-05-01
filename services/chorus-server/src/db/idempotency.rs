use async_trait::async_trait;
use sqlx::PgPool;
use uuid::Uuid;

use super::{DbError, IdempotencyOutcome, IdempotencyRepository};

/// PostgreSQL-backed idempotency repository.
pub struct PgIdempotencyRepository {
    pool: PgPool,
}

impl PgIdempotencyRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

/// PostgreSQL SQLSTATE for a statement timeout (`statement_timeout` exceeded).
const SQLSTATE_STATEMENT_TIMEOUT: &str = "57014";

fn map_sqlx_error(e: sqlx::Error) -> DbError {
    if let Some(code) = e.as_database_error().and_then(|d| d.code()) {
        if code == SQLSTATE_STATEMENT_TIMEOUT {
            return DbError::Timeout;
        }
    }
    DbError::Internal(anyhow::Error::from(e))
}

#[async_trait]
impl IdempotencyRepository for PgIdempotencyRepository {
    async fn begin(
        &self,
        api_key_id: Uuid,
        key: &str,
        request_hash: &[u8; 32],
        method: &str,
        path: &str,
    ) -> Result<IdempotencyOutcome, DbError> {
        let mut tx = self.pool.begin().await.map_err(map_sqlx_error)?;

        // 5-second cap on any statement in this transaction. Bounds the wait
        // when an in-progress row is locked by another request.
        sqlx::query("SET LOCAL statement_timeout = '5s'")
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx_error)?;

        // INSERT-or-recover. ON CONFLICT only succeeds if the existing row is
        // a stale in_progress row (>60s old) — otherwise it's a no-op and the
        // SELECT below reads the current row under FOR UPDATE.
        let inserted: Option<(Vec<u8>, String, Option<i16>, Option<Vec<u8>>)> = sqlx::query_as(
            "INSERT INTO idempotency_keys (api_key_id, idempotency_key, request_hash,
                                            request_method, request_path, status)
             VALUES ($1, $2, $3, $4, $5, 'in_progress')
             ON CONFLICT (api_key_id, idempotency_key) DO UPDATE
                SET status          = 'in_progress',
                    request_hash    = EXCLUDED.request_hash,
                    request_method  = EXCLUDED.request_method,
                    request_path    = EXCLUDED.request_path,
                    created_at      = now(),
                    expires_at      = now() + interval '24 hours',
                    response_status = NULL,
                    response_body   = NULL
              WHERE idempotency_keys.status = 'in_progress'
                AND idempotency_keys.created_at < now() - interval '60 seconds'
             RETURNING request_hash, status, response_status, response_body",
        )
        .bind(api_key_id)
        .bind(key)
        .bind(&request_hash[..])
        .bind(method)
        .bind(path)
        .fetch_optional(&mut *tx)
        .await
        .map_err(map_sqlx_error)?;

        let outcome = if inserted.is_some() {
            tx.commit().await.map_err(map_sqlx_error)?;
            IdempotencyOutcome::Fresh
        } else {
            let row: (Vec<u8>, String, Option<i16>, Option<Vec<u8>>) = sqlx::query_as(
                "SELECT request_hash, status, response_status, response_body
                 FROM idempotency_keys
                 WHERE api_key_id = $1 AND idempotency_key = $2
                 FOR UPDATE",
            )
            .bind(api_key_id)
            .bind(key)
            .fetch_one(&mut *tx)
            .await
            .map_err(map_sqlx_error)?;

            let (existing_hash, status, response_status, response_body) = row;

            let outcome = if existing_hash != request_hash[..] {
                IdempotencyOutcome::HashMismatch
            } else if status == "completed" {
                IdempotencyOutcome::Replay {
                    status: response_status.unwrap_or(0) as u16,
                    body: response_body.unwrap_or_default(),
                }
            } else {
                tx.rollback().await.ok();
                return Err(DbError::Internal(anyhow::anyhow!(
                    "idempotency: in_progress row returned from FOR UPDATE"
                )));
            };

            tx.commit().await.map_err(map_sqlx_error)?;
            outcome
        };

        Ok(outcome)
    }

    async fn complete(
        &self,
        api_key_id: Uuid,
        key: &str,
        response_status: u16,
        response_body: &[u8],
    ) -> Result<(), DbError> {
        sqlx::query(
            "UPDATE idempotency_keys
             SET status          = 'completed',
                 response_status = $3,
                 response_body   = $4
             WHERE api_key_id = $1 AND idempotency_key = $2",
        )
        .bind(api_key_id)
        .bind(key)
        .bind(response_status as i16)
        .bind(response_body)
        .execute(&self.pool)
        .await
        .map_err(map_sqlx_error)?;
        Ok(())
    }

    async fn delete_expired(&self, limit: i64) -> Result<u64, DbError> {
        let result = sqlx::query(
            "DELETE FROM idempotency_keys
             WHERE (api_key_id, idempotency_key) IN (
                 SELECT api_key_id, idempotency_key
                 FROM idempotency_keys
                 WHERE expires_at < now()
                 ORDER BY expires_at
                 LIMIT $1
             )",
        )
        .bind(limit)
        .execute(&self.pool)
        .await
        .map_err(map_sqlx_error)?;
        Ok(result.rows_affected())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::IdempotencyOutcome;
    use sqlx::PgPool;

    /// Seed an account + api_key row so FK on idempotency_keys is satisfied.
    async fn seed_api_key(pool: &PgPool) -> Uuid {
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

        key_id
    }

    fn h(byte: u8) -> [u8; 32] {
        [byte; 32]
    }

    #[sqlx::test(migrations = "./src/db/migrations")]
    async fn first_begin_returns_fresh(pool: PgPool) {
        let key_id = seed_api_key(&pool).await;
        let repo = PgIdempotencyRepository::new(pool);

        let outcome = repo
            .begin(key_id, "abc", &h(1), "POST", "/v1/sms/send")
            .await
            .unwrap();

        assert!(matches!(outcome, IdempotencyOutcome::Fresh));
    }

    #[sqlx::test(migrations = "./src/db/migrations")]
    async fn replay_after_complete_returns_cached_response(pool: PgPool) {
        let key_id = seed_api_key(&pool).await;
        let repo = PgIdempotencyRepository::new(pool);

        repo.begin(key_id, "abc", &h(1), "POST", "/v1/sms/send")
            .await
            .unwrap();
        repo.complete(key_id, "abc", 202, b"{\"message_id\":\"x\"}")
            .await
            .unwrap();

        let outcome = repo
            .begin(key_id, "abc", &h(1), "POST", "/v1/sms/send")
            .await
            .unwrap();
        match outcome {
            IdempotencyOutcome::Replay { status, body } => {
                assert_eq!(status, 202);
                assert_eq!(body, b"{\"message_id\":\"x\"}");
            }
            o => panic!("expected Replay, got {o:?}"),
        }
    }

    #[sqlx::test(migrations = "./src/db/migrations")]
    async fn different_hash_returns_hash_mismatch(pool: PgPool) {
        let key_id = seed_api_key(&pool).await;
        let repo = PgIdempotencyRepository::new(pool);

        repo.begin(key_id, "abc", &h(1), "POST", "/v1/sms/send")
            .await
            .unwrap();
        repo.complete(key_id, "abc", 202, b"ok").await.unwrap();

        let outcome = repo
            .begin(key_id, "abc", &h(2), "POST", "/v1/sms/send")
            .await
            .unwrap();
        assert!(matches!(outcome, IdempotencyOutcome::HashMismatch));
    }

    #[sqlx::test(migrations = "./src/db/migrations")]
    async fn stale_in_progress_row_recovers_to_fresh(pool: PgPool) {
        let key_id = seed_api_key(&pool).await;
        let repo = PgIdempotencyRepository::new(pool.clone());

        repo.begin(key_id, "abc", &h(1), "POST", "/v1/sms/send")
            .await
            .unwrap();

        // Backdate created_at by 90s to simulate a crashed holder.
        sqlx::query(
            "UPDATE idempotency_keys
             SET created_at = now() - interval '90 seconds'
             WHERE api_key_id = $1 AND idempotency_key = $2",
        )
        .bind(key_id)
        .bind("abc")
        .execute(&pool)
        .await
        .unwrap();

        let outcome = repo
            .begin(key_id, "abc", &h(2), "POST", "/v1/sms/send")
            .await
            .unwrap();
        assert!(matches!(outcome, IdempotencyOutcome::Fresh));
    }

    #[sqlx::test(migrations = "./src/db/migrations")]
    async fn delete_expired_removes_only_expired_rows(pool: PgPool) {
        let key_id = seed_api_key(&pool).await;
        let repo = PgIdempotencyRepository::new(pool.clone());

        for k in ["e1", "e2", "e3"] {
            repo.begin(key_id, k, &h(1), "POST", "/v1/sms/send")
                .await
                .unwrap();
            sqlx::query(
                "UPDATE idempotency_keys SET expires_at = now() - interval '1 second'
                 WHERE api_key_id=$1 AND idempotency_key=$2",
            )
            .bind(key_id)
            .bind(k)
            .execute(&pool)
            .await
            .unwrap();
        }
        for k in ["f1", "f2"] {
            repo.begin(key_id, k, &h(1), "POST", "/v1/sms/send")
                .await
                .unwrap();
        }

        let deleted = repo.delete_expired(100).await.unwrap();
        assert_eq!(deleted, 3);

        let remaining: i64 =
            sqlx::query_scalar("SELECT count(*) FROM idempotency_keys WHERE api_key_id = $1")
                .bind(key_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(remaining, 2);
    }

    #[sqlx::test(migrations = "./src/db/migrations")]
    async fn delete_expired_respects_limit(pool: PgPool) {
        let key_id = seed_api_key(&pool).await;
        let repo = PgIdempotencyRepository::new(pool.clone());

        for i in 0..5 {
            let k = format!("k{i}");
            repo.begin(key_id, &k, &h(1), "POST", "/v1/sms/send")
                .await
                .unwrap();
            sqlx::query(
                "UPDATE idempotency_keys SET expires_at = now() - interval '1 second'
                 WHERE api_key_id=$1 AND idempotency_key=$2",
            )
            .bind(key_id)
            .bind(&k)
            .execute(&pool)
            .await
            .unwrap();
        }

        let deleted = repo.delete_expired(2).await.unwrap();
        assert_eq!(deleted, 2);

        let remaining: i64 =
            sqlx::query_scalar("SELECT count(*) FROM idempotency_keys WHERE api_key_id = $1")
                .bind(key_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(remaining, 3);
    }

    #[sqlx::test(migrations = "./src/db/migrations")]
    async fn cascade_delete_on_api_key_removal(pool: PgPool) {
        let key_id = seed_api_key(&pool).await;
        let repo = PgIdempotencyRepository::new(pool.clone());

        repo.begin(key_id, "abc", &h(1), "POST", "/v1/sms/send")
            .await
            .unwrap();
        sqlx::query("DELETE FROM api_keys WHERE id = $1")
            .bind(key_id)
            .execute(&pool)
            .await
            .unwrap();

        let count: i64 =
            sqlx::query_scalar("SELECT count(*) FROM idempotency_keys WHERE api_key_id = $1")
                .bind(key_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(count, 0);
    }
}
