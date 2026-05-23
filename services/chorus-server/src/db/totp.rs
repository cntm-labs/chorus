use async_trait::async_trait;
use sqlx::PgPool;
use uuid::Uuid;

use super::{DbError, NewTotpUser, TotpRepository, TotpUser};

pub struct PgTotpRepository {
    pool: PgPool,
}

impl PgTotpRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

fn map_err(e: sqlx::Error) -> DbError {
    DbError::Internal(anyhow::Error::from(e))
}

#[async_trait]
impl TotpRepository for PgTotpRepository {
    async fn enroll(
        &self,
        new_user: &NewTotpUser,
        backup_code_hashes: &[Vec<u8>],
    ) -> Result<TotpUser, DbError> {
        let mut tx = self.pool.begin().await.map_err(map_err)?;

        let row: TotpUser = sqlx::query_as(
            "INSERT INTO totp_users
                (account_id, user_id, secret, status, issuer, label)
             VALUES ($1, $2, $3, 'pending', $4, $5)
             RETURNING *",
        )
        .bind(new_user.account_id)
        .bind(&new_user.user_id)
        .bind(&new_user.encrypted_secret)
        .bind(new_user.issuer.as_deref())
        .bind(new_user.label.as_deref())
        .fetch_one(&mut *tx)
        .await
        .map_err(map_err)?;

        for hash in backup_code_hashes {
            sqlx::query(
                "INSERT INTO totp_backup_codes (account_id, user_id, code_hash)
                 VALUES ($1, $2, $3)",
            )
            .bind(new_user.account_id)
            .bind(&new_user.user_id)
            .bind(hash.as_slice())
            .execute(&mut *tx)
            .await
            .map_err(map_err)?;
        }

        tx.commit().await.map_err(map_err)?;
        Ok(row)
    }

    async fn find(
        &self,
        account_id: Uuid,
        user_id: &str,
    ) -> Result<Option<TotpUser>, DbError> {
        let row: Option<TotpUser> = sqlx::query_as(
            "SELECT * FROM totp_users WHERE account_id = $1 AND user_id = $2",
        )
        .bind(account_id)
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(map_err)?;
        Ok(row)
    }

    async fn activate(&self, account_id: Uuid, user_id: &str) -> Result<(), DbError> {
        let result = sqlx::query(
            "UPDATE totp_users
             SET status='active', activated_at=now(), updated_at=now()
             WHERE account_id=$1 AND user_id=$2 AND status='pending'",
        )
        .bind(account_id)
        .bind(user_id)
        .execute(&self.pool)
        .await
        .map_err(map_err)?;
        if result.rows_affected() == 0 {
            return Err(DbError::NotFound);
        }
        Ok(())
    }

    async fn touch_last_verified(
        &self,
        account_id: Uuid,
        user_id: &str,
    ) -> Result<(), DbError> {
        sqlx::query(
            "UPDATE totp_users SET last_verified_at=now(), updated_at=now()
             WHERE account_id=$1 AND user_id=$2 AND status='active'",
        )
        .bind(account_id)
        .bind(user_id)
        .execute(&self.pool)
        .await
        .map_err(map_err)?;
        Ok(())
    }

    async fn disenroll(&self, account_id: Uuid, user_id: &str) -> Result<bool, DbError> {
        // Zero out the secret with a single 0x00 byte (cannot be empty: secret is NOT NULL).
        let zero_byte: &[u8] = &[0u8];
        let result = sqlx::query(
            "UPDATE totp_users
             SET status='disabled', secret=$3, updated_at=now()
             WHERE account_id=$1 AND user_id=$2 AND status IN ('pending','active')",
        )
        .bind(account_id)
        .bind(user_id)
        .bind(zero_byte)
        .execute(&self.pool)
        .await
        .map_err(map_err)?;
        Ok(result.rows_affected() > 0)
    }

    async fn consume_backup_code(
        &self,
        account_id: Uuid,
        user_id: &str,
        code_hash: &[u8],
    ) -> Result<bool, DbError> {
        let row: Option<(i64,)> = sqlx::query_as(
            "UPDATE totp_backup_codes
             SET used_at = now()
             WHERE account_id=$1 AND user_id=$2 AND code_hash=$3 AND used_at IS NULL
             RETURNING id",
        )
        .bind(account_id)
        .bind(user_id)
        .bind(code_hash)
        .fetch_optional(&self.pool)
        .await
        .map_err(map_err)?;
        Ok(row.is_some())
    }

    async fn unused_backup_codes_count(
        &self,
        account_id: Uuid,
        user_id: &str,
    ) -> Result<i64, DbError> {
        let n: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM totp_backup_codes
             WHERE account_id=$1 AND user_id=$2 AND used_at IS NULL",
        )
        .bind(account_id)
        .bind(user_id)
        .fetch_one(&self.pool)
        .await
        .map_err(map_err)?;
        Ok(n)
    }

    async fn replace_backup_codes(
        &self,
        account_id: Uuid,
        user_id: &str,
        new_hashes: &[Vec<u8>],
    ) -> Result<(), DbError> {
        let mut tx = self.pool.begin().await.map_err(map_err)?;
        sqlx::query(
            "DELETE FROM totp_backup_codes WHERE account_id=$1 AND user_id=$2",
        )
        .bind(account_id)
        .bind(user_id)
        .execute(&mut *tx)
        .await
        .map_err(map_err)?;
        for hash in new_hashes {
            sqlx::query(
                "INSERT INTO totp_backup_codes (account_id, user_id, code_hash)
                 VALUES ($1, $2, $3)",
            )
            .bind(account_id)
            .bind(user_id)
            .bind(hash.as_slice())
            .execute(&mut *tx)
            .await
            .map_err(map_err)?;
        }
        tx.commit().await.map_err(map_err)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{NewTotpUser, TotpRepository};

    async fn seed_account(pool: &PgPool) -> Uuid {
        let id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO accounts (id, name, owner_email, is_active)
             VALUES ($1, 'test', 't@x.com', true)",
        )
        .bind(id)
        .execute(pool)
        .await
        .unwrap();
        id
    }

    fn fixture(acct: Uuid, user_id: &str) -> NewTotpUser {
        NewTotpUser {
            account_id: acct,
            user_id: user_id.to_string(),
            encrypted_secret: vec![0xAA; 48],
            issuer: Some("Acme".into()),
            label: Some("Acme:test".into()),
        }
    }

    fn hashes(n: usize) -> Vec<Vec<u8>> {
        (0..n).map(|i| vec![i as u8; 32]).collect()
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./src/db/migrations")]
    async fn enroll_creates_pending_user_with_backup_codes(pool: PgPool) {
        let acct = seed_account(&pool).await;
        let repo = PgTotpRepository::new(pool.clone());
        let u = repo.enroll(&fixture(acct, "alice"), &hashes(10)).await.unwrap();
        assert_eq!(u.status, "pending");
        assert_eq!(u.user_id, "alice");
        let cnt = repo.unused_backup_codes_count(acct, "alice").await.unwrap();
        assert_eq!(cnt, 10);
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./src/db/migrations")]
    async fn enroll_errors_when_user_already_active(pool: PgPool) {
        let acct = seed_account(&pool).await;
        let repo = PgTotpRepository::new(pool);
        repo.enroll(&fixture(acct, "alice"), &hashes(1)).await.unwrap();
        let err = repo.enroll(&fixture(acct, "alice"), &hashes(1)).await.unwrap_err();
        assert!(matches!(err, DbError::Internal(_)));
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./src/db/migrations")]
    async fn find_returns_none_for_unknown(pool: PgPool) {
        let acct = seed_account(&pool).await;
        let repo = PgTotpRepository::new(pool);
        assert!(repo.find(acct, "ghost").await.unwrap().is_none());
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./src/db/migrations")]
    async fn find_scopes_to_account(pool: PgPool) {
        let acct = seed_account(&pool).await;
        let repo = PgTotpRepository::new(pool);
        repo.enroll(&fixture(acct, "alice"), &hashes(1)).await.unwrap();
        let other = Uuid::new_v4();
        assert!(repo.find(other, "alice").await.unwrap().is_none());
        assert!(repo.find(acct, "alice").await.unwrap().is_some());
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./src/db/migrations")]
    async fn activate_pending_to_active(pool: PgPool) {
        let acct = seed_account(&pool).await;
        let repo = PgTotpRepository::new(pool);
        repo.enroll(&fixture(acct, "alice"), &hashes(1)).await.unwrap();
        repo.activate(acct, "alice").await.unwrap();
        let u = repo.find(acct, "alice").await.unwrap().unwrap();
        assert_eq!(u.status, "active");
        assert!(u.activated_at.is_some());
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./src/db/migrations")]
    async fn activate_errors_when_not_pending(pool: PgPool) {
        let acct = seed_account(&pool).await;
        let repo = PgTotpRepository::new(pool);
        repo.enroll(&fixture(acct, "alice"), &hashes(1)).await.unwrap();
        repo.activate(acct, "alice").await.unwrap();
        let err = repo.activate(acct, "alice").await.unwrap_err();
        assert!(matches!(err, DbError::NotFound));
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./src/db/migrations")]
    async fn disenroll_clears_secret(pool: PgPool) {
        let acct = seed_account(&pool).await;
        let repo = PgTotpRepository::new(pool.clone());
        repo.enroll(&fixture(acct, "alice"), &hashes(10)).await.unwrap();
        assert!(repo.disenroll(acct, "alice").await.unwrap());

        let u = repo.find(acct, "alice").await.unwrap().unwrap();
        assert_eq!(u.status, "disabled");
        assert_eq!(u.secret, vec![0x00]);
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./src/db/migrations")]
    async fn consume_backup_code_marks_used_and_returns_true(pool: PgPool) {
        let acct = seed_account(&pool).await;
        let repo = PgTotpRepository::new(pool);
        let h = vec![0xBB; 32];
        repo.enroll(&fixture(acct, "alice"), std::slice::from_ref(&h)).await.unwrap();
        assert!(repo.consume_backup_code(acct, "alice", &h).await.unwrap());
        assert!(!repo.consume_backup_code(acct, "alice", &h).await.unwrap());
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./src/db/migrations")]
    async fn consume_backup_code_returns_false_when_unknown(pool: PgPool) {
        let acct = seed_account(&pool).await;
        let repo = PgTotpRepository::new(pool);
        repo.enroll(&fixture(acct, "alice"), &hashes(1)).await.unwrap();
        let other = vec![0xCC; 32];
        assert!(!repo.consume_backup_code(acct, "alice", &other).await.unwrap());
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./src/db/migrations")]
    async fn unused_backup_codes_count_excludes_used(pool: PgPool) {
        let acct = seed_account(&pool).await;
        let repo = PgTotpRepository::new(pool);
        let hs = hashes(10);
        repo.enroll(&fixture(acct, "alice"), &hs).await.unwrap();
        repo.consume_backup_code(acct, "alice", &hs[0]).await.unwrap();
        repo.consume_backup_code(acct, "alice", &hs[1]).await.unwrap();
        assert_eq!(repo.unused_backup_codes_count(acct, "alice").await.unwrap(), 8);
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./src/db/migrations")]
    async fn replace_backup_codes_atomically_swaps(pool: PgPool) {
        let acct = seed_account(&pool).await;
        let repo = PgTotpRepository::new(pool);
        let original = hashes(10);
        repo.enroll(&fixture(acct, "alice"), &original).await.unwrap();
        repo.consume_backup_code(acct, "alice", &original[0]).await.unwrap();

        let fresh: Vec<Vec<u8>> = (10..20).map(|i| vec![i as u8; 32]).collect();
        repo.replace_backup_codes(acct, "alice", &fresh).await.unwrap();

        // Old codes (including used) gone
        assert!(!repo.consume_backup_code(acct, "alice", &original[1]).await.unwrap());
        // New codes available
        assert!(repo.consume_backup_code(acct, "alice", &fresh[0]).await.unwrap());
        assert_eq!(repo.unused_backup_codes_count(acct, "alice").await.unwrap(), 9);
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./src/db/migrations")]
    async fn cascade_delete_on_account_removal_drops_user_and_codes(pool: PgPool) {
        let acct = seed_account(&pool).await;
        let repo = PgTotpRepository::new(pool.clone());
        repo.enroll(&fixture(acct, "alice"), &hashes(10)).await.unwrap();
        sqlx::query("DELETE FROM accounts WHERE id = $1")
            .bind(acct)
            .execute(&pool)
            .await
            .unwrap();
        let n: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM totp_users WHERE account_id = $1",
        )
        .bind(acct)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(n, 0);
    }
}
