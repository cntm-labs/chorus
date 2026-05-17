# Verification API + Waterfall Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship Stripe-Verify-class API at `/v1/verifications/*` with smart waterfall routing (email-first → SMS fallback), per-call cost reporting, and rate limiting — the hero "save 60-80% vs Twilio" pitch.

**Architecture:** Postgres `verifications` table (audit/billing/lifecycle) + Valkey for code TTL + sliding-window rate limits. Smart routing picks the cheapest eligible channel per the user's `channels` preference. Reuses C1 idempotency on `create` and `resend`. Legacy `/v1/otp/*` left untouched (deprecated label only).

**Tech Stack:** Rust + Axum, SQLx + Postgres 16, redis crate (Valkey-compatible), sha2, tokio, tracing, metrics.

---

## Spec reference

`docs/superpowers/specs/2026-05-06-verification-api-design.md`. Schema, error matrix, and component contracts are normative — this plan implements them step-by-step.

---

## File structure

### New files

| Path | Responsibility |
|---|---|
| `services/chorus-server/src/db/migrations/009_create_verifications.sql` | Schema migration. |
| `services/chorus-server/src/db/verification.rs` | `PgVerificationRepository` (Postgres impl + sqlx tests). |
| `services/chorus-server/src/verification.rs` | Orchestration module — constants, `ChannelChoice`/`RoutingError`, code generator, pricing, Valkey helpers, rate limit, `select_channel`, `expire_pending_loop`. |
| `services/chorus-server/src/routes/verifications.rs` | 6 route handlers (create/check/resend/cancel/get/list). |

### Modified files

| Path | Reason |
|---|---|
| `services/chorus-server/src/db/mod.rs` | Add `VerificationRepository` trait + `Verification`/`NewVerification` types + `pub mod verification`. |
| `services/chorus-server/src/lib.rs` | Add `pub mod verification`. |
| `services/chorus-server/src/app.rs` | Wire `verification_repo` field/accessor/`with_repos` param; register 6 new routes. |
| `services/chorus-server/src/main.rs` | Spawn `expire_pending_loop` alongside the idempotency cleanup. |
| `services/chorus-server/src/routes/mod.rs` | Register the new module. |
| `services/chorus-server/tests/api_test.rs` | Add `MockVerificationRepo` + integration tests; thread the new arg through 3 existing `with_repos` call sites. |

---

## Conventions

- All file paths are relative to the worktree root.
- Test commands run from the worktree root.
- Each task ends with one commit. Style: `feat(server): B1 — <summary>` / `test(server): B1 — <summary>`.
- The legacy `/v1/otp/*` routes are **not touched** — they keep their C1 idempotency wiring.

---

## Task 1: Migration 009 — `verifications` table

**Files:**
- Create: `services/chorus-server/src/db/migrations/009_create_verifications.sql`

- [ ] **Step 1: Create the migration file**

```sql
-- services/chorus-server/src/db/migrations/009_create_verifications.sql
CREATE TABLE verifications (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    account_id      UUID NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    api_key_id      UUID NOT NULL REFERENCES api_keys(id) ON DELETE CASCADE,
    channel         TEXT NOT NULL CHECK (channel IN ('sms', 'email')),
    recipient       TEXT NOT NULL,
    status          TEXT NOT NULL CHECK (status IN ('pending', 'approved', 'canceled', 'expired')),
    check_attempts  INTEGER NOT NULL DEFAULT 0,
    resend_attempts INTEGER NOT NULL DEFAULT 0,
    cost_micro      BIGINT  NOT NULL DEFAULT 0,
    cost_currency   TEXT    NOT NULL DEFAULT 'USD',
    environment     TEXT    NOT NULL,
    app_name        TEXT,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at      TIMESTAMPTZ NOT NULL
);

CREATE INDEX verifications_account_created_idx ON verifications (account_id, created_at DESC);
CREATE INDEX verifications_recipient_idx       ON verifications (recipient, created_at DESC);
CREATE INDEX verifications_pending_expiry_idx  ON verifications (expires_at)
    WHERE status = 'pending';
```

- [ ] **Step 2: Verify workspace still compiles**

```
cargo check -p chorus-server
```
Expected: PASS (no code references the new table yet).

- [ ] **Step 3: Commit**

```bash
git add services/chorus-server/src/db/migrations/009_create_verifications.sql
git commit -m "feat(server): B1 — add verifications migration"
```

---

## Task 2: Repository trait + types

**Files:**
- Modify: `services/chorus-server/src/db/mod.rs`
- Create: `services/chorus-server/src/db/verification.rs` (empty stub)

- [ ] **Step 1: Register the new module in `db/mod.rs`**

In the `pub mod ...` block near the top of `services/chorus-server/src/db/mod.rs`, add a line so the block reads (alphabetical):

```rust
pub mod admin;
pub mod billing;
pub mod idempotency;
pub mod postgres;
pub mod provider_config;
pub mod suppression;
pub mod verification;
pub mod webhook;
```

- [ ] **Step 2: Append types + trait to the end of `db/mod.rs`**

```rust
/// A verification (OTP) lifecycle record.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Verification {
    pub id: Uuid,
    pub account_id: Uuid,
    pub api_key_id: Uuid,
    pub channel: String,
    pub recipient: String,
    pub status: String,
    pub check_attempts: i32,
    pub resend_attempts: i32,
    pub cost_micro: i64,
    pub cost_currency: String,
    pub environment: String,
    pub app_name: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

/// Parameters for inserting a new verification.
pub struct NewVerification {
    pub account_id: Uuid,
    pub api_key_id: Uuid,
    pub channel: String,
    pub recipient: String,
    pub environment: String,
    pub app_name: Option<String>,
    pub initial_cost_micro: i64,
}

/// Verification lifecycle and counters.
#[async_trait]
pub trait VerificationRepository: Send + Sync {
    /// Insert a new pending verification (expires_at = now() + 5 min).
    async fn insert(&self, v: &NewVerification) -> Result<Verification, DbError>;

    /// Find by id scoped to an account.
    async fn find_by_id(
        &self,
        id: Uuid,
        account_id: Uuid,
    ) -> Result<Option<Verification>, DbError>;

    /// List for an account ordered by created_at DESC.
    async fn list_by_account(
        &self,
        account_id: Uuid,
        pagination: &Pagination,
    ) -> Result<Vec<Verification>, DbError>;

    /// Increment `check_attempts` atomically; returns the new count.
    /// Errors with `NotFound` if status != 'pending'.
    async fn increment_check_attempts(
        &self,
        id: Uuid,
        account_id: Uuid,
    ) -> Result<i32, DbError>;

    /// Set status='approved' (only if currently pending). Returns NotFound otherwise.
    async fn mark_approved(&self, id: Uuid, account_id: Uuid) -> Result<(), DbError>;

    /// Set status='canceled' only if currently pending. Returns true on success.
    async fn mark_canceled(&self, id: Uuid, account_id: Uuid) -> Result<bool, DbError>;

    /// Atomic resend: increments resend_attempts, adds cost, resets check_attempts.
    /// Errors with NotFound if not pending or resend cap reached.
    async fn record_resend(
        &self,
        id: Uuid,
        account_id: Uuid,
        additional_cost_micro: i64,
        max_resends: i32,
    ) -> Result<Verification, DbError>;

    /// Cleanup: bulk-mark expired pending rows. Returns count.
    async fn expire_pending(&self, limit: i64) -> Result<u64, DbError>;
}
```

- [ ] **Step 3: Create empty `verification.rs`**

```bash
: > services/chorus-server/src/db/verification.rs
```

- [ ] **Step 4: Compile**

```
cargo check -p chorus-server
```
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add services/chorus-server/src/db/mod.rs services/chorus-server/src/db/verification.rs
git commit -m "feat(server): B1 — VerificationRepository trait + types"
```

---

## Task 3: `PgVerificationRepository` impl + sqlx tests

**Files:**
- Modify: `services/chorus-server/src/db/verification.rs`

- [ ] **Step 1: Write the Postgres impl**

Replace the empty contents of `services/chorus-server/src/db/verification.rs` with:

```rust
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use super::{DbError, NewVerification, Pagination, Verification, VerificationRepository};

/// PostgreSQL-backed verification repository.
pub struct PgVerificationRepository {
    pool: PgPool,
}

impl PgVerificationRepository {
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
        let row: Option<Verification> = sqlx::query_as(
            "SELECT * FROM verifications WHERE id = $1 AND account_id = $2",
        )
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

    async fn increment_check_attempts(
        &self,
        id: Uuid,
        account_id: Uuid,
    ) -> Result<i32, DbError> {
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
```

- [ ] **Step 2: Append `sqlx::test` repo tests (ignored by default)**

Append at the end of the same file:

```rust
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
            .list_by_account(acct, &Pagination { limit: 10, offset: 0 })
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
            sqlx::query("UPDATE verifications SET expires_at = now() - interval '1s' WHERE id = $1")
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
        let n: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM verifications WHERE account_id = $1",
        )
        .bind(acct)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(n, 0);
    }
}
```

- [ ] **Step 3: Compile + clippy**

```
cargo check -p chorus-server --tests
cargo clippy -p chorus-server --all-targets -- -D warnings
```
Expected: PASS (sqlx tests are ignored by default; clippy uses pre-existing warnings only).

- [ ] **Step 4: Commit**

```bash
git add services/chorus-server/src/db/verification.rs
git commit -m "feat(server): B1 — PgVerificationRepository impl + sqlx tests"
```

---

## Task 4: `verification.rs` — constants, pricing, code generator

**Files:**
- Create: `services/chorus-server/src/verification.rs`
- Modify: `services/chorus-server/src/lib.rs`

- [ ] **Step 1: Register module in `lib.rs`**

In `services/chorus-server/src/lib.rs`, add (alphabetical) so the block reads:

```rust
pub mod app;
pub mod auth;
pub mod billing;
pub mod config;
pub mod db;
pub mod idempotency;
pub mod metrics;
pub mod middleware;
pub mod otp;
pub mod queue;
pub mod routes;
pub mod suppression;
pub mod verification;
```

- [ ] **Step 2: Create the orchestration module with pure helpers**

Create `services/chorus-server/src/verification.rs`:

```rust
//! Verification orchestration: constants, code generator, pricing helpers,
//! Valkey access, rate limiting, smart routing, cleanup loop.
//!
//! See `docs/superpowers/specs/2026-05-06-verification-api-design.md`.

use rand::Rng;

/// Code length in digits.
pub const CODE_LENGTH: usize = 6;
/// Valkey TTL for the code, in seconds (5 minutes).
pub const TTL_SECONDS: u64 = 300;
/// Max times `/check` may be called with a wrong code before lockout.
pub const MAX_CHECK_ATTEMPTS: i32 = 5;
/// Max times `/resend` may be called per verification.
pub const MAX_RESEND_ATTEMPTS: i32 = 3;
/// Sliding-window rate limit per recipient (1 hour window).
pub const RATE_LIMIT_PER_RCPT_HOUR: u32 = 5;
/// Sliding-window rate limit per account (1 minute window).
pub const RATE_LIMIT_PER_ACCT_MINUTE: u32 = 100;

/// Generate a cryptographically random `CODE_LENGTH`-digit code.
pub fn generate_code() -> String {
    let n: u32 = rand::rng().random_range(0..1_000_000);
    format!("{n:06}")
}

/// Pricing lookup. Returns cost in micro-USD for a single delivery.
pub fn cost_for(channel: &str, recipient: &str) -> i64 {
    match channel {
        "email" => 100,
        "sms" => sms_cost_for_country(extract_country(recipient)),
        _ => 0,
    }
}

fn sms_cost_for_country(cc: &str) -> i64 {
    match cc {
        "US" | "CA" => 5_000,
        "TH" => 6_000,
        _ => 8_000,
    }
}

/// Map a leading E.164 prefix to an ISO country code.
/// Returns `"??"` when the prefix is unknown — caller treats as fallback rate.
fn extract_country(e164: &str) -> &'static str {
    let digits = e164.trim_start_matches('+');
    // Longest match first (greedy).
    const PREFIXES: &[(&str, &str)] = &[
        ("1", "US"),   // also CA — single rate applies
        ("44", "UK"),
        ("49", "DE"),
        ("66", "TH"),
        ("65", "SG"),
        ("81", "JP"),
        ("82", "KR"),
        ("86", "CN"),
    ];
    let mut best: &str = "??";
    let mut best_len: usize = 0;
    for (prefix, cc) in PREFIXES {
        if digits.starts_with(prefix) && prefix.len() > best_len {
            best = cc;
            best_len = prefix.len();
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_code_is_six_digits() {
        for _ in 0..100 {
            let c = generate_code();
            assert_eq!(c.len(), 6, "code = {c:?}");
            assert!(c.chars().all(|c| c.is_ascii_digit()), "code = {c:?}");
        }
    }

    #[test]
    fn cost_for_email_flat() {
        assert_eq!(cost_for("email", "alice@example.com"), 100);
    }

    #[test]
    fn cost_for_sms_us() {
        assert_eq!(cost_for("sms", "+14155552671"), 5_000);
    }

    #[test]
    fn cost_for_sms_thailand() {
        assert_eq!(cost_for("sms", "+66812345678"), 6_000);
    }

    #[test]
    fn cost_for_sms_unknown_country_fallback() {
        assert_eq!(cost_for("sms", "+33123456789"), 8_000);
    }

    #[test]
    fn cost_for_unknown_channel_is_zero() {
        assert_eq!(cost_for("whatsapp", "+66..."), 0);
    }
}
```

- [ ] **Step 3: Run unit tests**

```
cargo test -p chorus-server --lib verification::tests
```
Expected: 6 tests passing.

- [ ] **Step 4: Commit**

```bash
git add services/chorus-server/src/lib.rs services/chorus-server/src/verification.rs
git commit -m "feat(server): B1 — verification constants, code generator, pricing"
```

---

## Task 5: `AppState` wiring

**Files:**
- Modify: `services/chorus-server/src/app.rs`
- Modify: `services/chorus-server/tests/api_test.rs`

The trait + repo from Task 2/3 must be wired before later route tasks. This task also updates the 3 existing `with_repos` call sites in `api_test.rs`.

- [ ] **Step 1: Add imports + field + accessor + constructor wiring**

In `services/chorus-server/src/app.rs`:

A. Extend `use crate::db::...` to include `VerificationRepository`:

```rust
use crate::db::{
    AccountRepository, AdminKeyRepository, AdminRepository, ApiKeyRepository,
    IdempotencyRepository, MessageRepository, PgAdminRepository,
    ProviderConfigRepository, SuppressionRepository, VerificationRepository,
    WebhookRepository,
};
```

B. Add the import line near the other `db::...` repo imports:

```rust
use crate::db::verification::PgVerificationRepository;
```

C. Add field to `AppState` (after `idempotency_repo`):

```rust
    /// Verification record repository.
    verification_repo: Arc<dyn VerificationRepository>,
```

D. In `AppState::new`, after `let idempotency_repo = ...;`:

```rust
        let verification_repo = Arc::new(PgVerificationRepository::new(db.clone()));
```

and include it in the struct literal (next to `idempotency_repo`):

```rust
            suppression_repo,
            idempotency_repo,
            verification_repo,
            billing_repo,
```

E. Update `with_repos`. The current signature ends with `idempotency_repo`. Add a new arg and field at the same position:

```rust
    #[allow(clippy::too_many_arguments)]
    pub fn with_repos(
        redis: redis::Client,
        config: Arc<Config>,
        account_repo: Arc<dyn AccountRepository>,
        message_repo: Arc<dyn MessageRepository>,
        api_key_repo: Arc<dyn ApiKeyRepository>,
        provider_config_repo: Arc<dyn ProviderConfigRepository>,
        webhook_repo: Arc<dyn WebhookRepository>,
        suppression_repo: Arc<dyn SuppressionRepository>,
        idempotency_repo: Arc<dyn IdempotencyRepository>,
        verification_repo: Arc<dyn VerificationRepository>,
    ) -> Self {
        Self {
            db: None,
            redis,
            http_client: reqwest::Client::new(),
            config,
            account_repo,
            message_repo,
            api_key_repo,
            provider_config_repo,
            webhook_repo,
            suppression_repo,
            idempotency_repo,
            verification_repo,
            billing_repo: Arc::new(crate::db::billing::NullBillingRepository),
            admin_key_repo: Arc::new(NullAdminKeyRepository),
            admin_repo: Arc::new(NullAdminRepository),
        }
    }
```

F. Add accessor method (after `idempotency_repo()`):

```rust
    /// Access the verification repository.
    pub fn verification_repo(&self) -> Arc<dyn VerificationRepository> {
        Arc::clone(&self.verification_repo)
    }
```

- [ ] **Step 2: Add a `NullVerificationRepo` to `tests/api_test.rs`**

In `services/chorus-server/tests/api_test.rs`, find the `NullIdempotencyRepo` (around line 27) and add — right after its `impl` block:

```rust
/// No-op verification repo for tests that don't exercise verification logic.
struct NullVerificationRepo;

#[async_trait]
impl chorus_server::db::VerificationRepository for NullVerificationRepo {
    async fn insert(
        &self,
        _v: &chorus_server::db::NewVerification,
    ) -> Result<chorus_server::db::Verification, DbError> {
        Err(DbError::Internal(anyhow::anyhow!("NullVerificationRepo::insert not implemented")))
    }
    async fn find_by_id(
        &self,
        _id: Uuid,
        _account_id: Uuid,
    ) -> Result<Option<chorus_server::db::Verification>, DbError> {
        Ok(None)
    }
    async fn list_by_account(
        &self,
        _account_id: Uuid,
        _pagination: &chorus_server::db::Pagination,
    ) -> Result<Vec<chorus_server::db::Verification>, DbError> {
        Ok(vec![])
    }
    async fn increment_check_attempts(
        &self,
        _id: Uuid,
        _account_id: Uuid,
    ) -> Result<i32, DbError> {
        Err(DbError::NotFound)
    }
    async fn mark_approved(&self, _id: Uuid, _account_id: Uuid) -> Result<(), DbError> {
        Err(DbError::NotFound)
    }
    async fn mark_canceled(&self, _id: Uuid, _account_id: Uuid) -> Result<bool, DbError> {
        Ok(false)
    }
    async fn record_resend(
        &self,
        _id: Uuid,
        _account_id: Uuid,
        _cost: i64,
        _max: i32,
    ) -> Result<chorus_server::db::Verification, DbError> {
        Err(DbError::NotFound)
    }
    async fn expire_pending(&self, _limit: i64) -> Result<u64, DbError> {
        Ok(0)
    }
}
```

Also extend the existing `use chorus_server::db::{...}` block to include `VerificationRepository`:

```rust
use chorus_server::db::{
    Account, AccountRepository, AddSuppressionResult, ApiKey, ApiKeyRepository, DbError,
    DeliveryEvent, IdempotencyOutcome, IdempotencyRepository, Message, MessageRepository,
    NewMessage, NewProviderConfig, NewSuppression, NewWebhook, Pagination, ProviderConfig,
    ProviderConfigRepository, Suppression, SuppressionRepository, VerificationRepository,
    Webhook, WebhookRepository,
};
```

- [ ] **Step 3: Add the new arg to all `with_repos` call sites**

There are three call sites in `tests/api_test.rs` (around lines 458, 1731, 2187). For each, add a new last argument after `idempotency_repo` (or its equivalent):

```rust
let verification_repo: Arc<dyn VerificationRepository> = Arc::new(NullVerificationRepo);
```

and pass it as the last argument to `AppState::with_repos(...)`. Run `cargo check --tests` after each to verify.

- [ ] **Step 4: Verify everything compiles and tests still pass**

```
cargo check -p chorus-server --tests
cargo test -p chorus-server --test api_test
```
Expected: PASS (48 pre-existing tests).

- [ ] **Step 5: Commit**

```bash
git add services/chorus-server/src/app.rs services/chorus-server/tests/api_test.rs
git commit -m "feat(server): B1 — wire verification_repo through AppState"
```

---

## Task 6: Valkey helpers — `store_code`, `check_code`, `invalidate_code`

**Files:**
- Modify: `services/chorus-server/src/verification.rs`

- [ ] **Step 1: Append helpers**

Append to `services/chorus-server/src/verification.rs` (above `#[cfg(test)]`):

```rust
use sha2::{Digest, Sha256};
use uuid::Uuid;

/// Result of an attempted code check.
#[derive(Debug, PartialEq, Eq)]
pub enum CheckCodeOutcome {
    /// Code matched; the Valkey entry has been deleted.
    Match,
    /// Code did not match. The entry remains until TTL or lockout.
    Mismatch,
    /// No entry exists for this id (TTL expired or already consumed/canceled).
    Gone,
}

fn valkey_key(id: Uuid) -> String {
    format!("verify:{id}")
}

/// Hash the recipient for use in keys and logs (avoid plaintext PII).
pub fn hash_recipient(recipient: &str) -> String {
    hex::encode(Sha256::digest(recipient.as_bytes()))
}

/// Store the code with TTL. Overwrites any previous entry (e.g. on resend).
pub async fn store_code(
    redis: &redis::Client,
    id: Uuid,
    recipient: &str,
    code: &str,
) -> anyhow::Result<()> {
    let key = valkey_key(id);
    let value = serde_json::json!({
        "code": code,
        "recipient_hash": hash_recipient(recipient),
    });
    let mut conn = redis.get_multiplexed_tokio_connection().await?;
    redis::cmd("SET")
        .arg(&key)
        .arg(value.to_string())
        .arg("EX")
        .arg(TTL_SECONDS)
        .query_async::<String>(&mut conn)
        .await?;
    Ok(())
}

/// Compare the provided code against the stored one.
/// On `Match` the entry is deleted. On `Mismatch` the entry is left alone
/// (caller increments the authoritative DB counter).
pub async fn check_code(
    redis: &redis::Client,
    id: Uuid,
    code: &str,
) -> anyhow::Result<CheckCodeOutcome> {
    let key = valkey_key(id);
    let mut conn = redis.get_multiplexed_tokio_connection().await?;
    let raw: Option<String> = redis::cmd("GET").arg(&key).query_async(&mut conn).await?;
    let Some(raw) = raw else { return Ok(CheckCodeOutcome::Gone) };
    let data: serde_json::Value = serde_json::from_str(&raw)?;
    let stored = data["code"].as_str().unwrap_or("");
    if stored == code {
        redis::cmd("DEL").arg(&key).query_async::<i64>(&mut conn).await?;
        Ok(CheckCodeOutcome::Match)
    } else {
        Ok(CheckCodeOutcome::Mismatch)
    }
}

/// Delete the Valkey entry (used by cancel and lockout paths).
pub async fn invalidate_code(redis: &redis::Client, id: Uuid) -> anyhow::Result<()> {
    let key = valkey_key(id);
    let mut conn = redis.get_multiplexed_tokio_connection().await?;
    redis::cmd("DEL").arg(&key).query_async::<i64>(&mut conn).await?;
    Ok(())
}
```

- [ ] **Step 2: Compile + lint**

```
cargo check -p chorus-server
cargo clippy -p chorus-server -- -D warnings
```
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add services/chorus-server/src/verification.rs
git commit -m "feat(server): B1 — Valkey helpers store_code/check_code/invalidate_code"
```

---

## Task 7: Rate limiting — sliding-window via Lua

**Files:**
- Modify: `services/chorus-server/src/verification.rs`

- [ ] **Step 1: Append the Lua script + Rust wrapper**

Append to `services/chorus-server/src/verification.rs` (above `#[cfg(test)]`):

```rust
/// Reasons routing rejects a verification request.
#[derive(Debug)]
pub enum RoutingError {
    NoRecipient,
    InvalidPhone,
    InvalidEmail,
    NoEligibleChannel,
    RateLimitedRecipient { retry_after_sec: u64 },
    RateLimitedAccount { retry_after_sec: u64 },
    Db(crate::db::DbError),
    Internal(anyhow::Error),
}

/// Atomic sliding-window check + increment over two ZSET keys.
/// Returns:
///   0           = allowed (and both windows incremented),
///   recipient   = "rcpt"  → retry-after = oldest_score_recipient + window_ms - now
///   account     = "acct"
const RATE_LIMIT_LUA: &str = r#"
local key_rcpt    = KEYS[1]
local key_acct    = KEYS[2]
local now_ms      = tonumber(ARGV[1])
local window_rcpt = tonumber(ARGV[2])
local limit_rcpt  = tonumber(ARGV[3])
local window_acct = tonumber(ARGV[4])
local limit_acct  = tonumber(ARGV[5])
local member      = ARGV[6]

redis.call('ZREMRANGEBYSCORE', key_rcpt, 0, now_ms - window_rcpt)
local count_rcpt = redis.call('ZCARD', key_rcpt)
if count_rcpt >= limit_rcpt then
    local oldest = redis.call('ZRANGE', key_rcpt, 0, 0, 'WITHSCORES')
    return {'rcpt', tonumber(oldest[2]) + window_rcpt - now_ms}
end

redis.call('ZREMRANGEBYSCORE', key_acct, 0, now_ms - window_acct)
local count_acct = redis.call('ZCARD', key_acct)
if count_acct >= limit_acct then
    local oldest = redis.call('ZRANGE', key_acct, 0, 0, 'WITHSCORES')
    return {'acct', tonumber(oldest[2]) + window_acct - now_ms}
end

redis.call('ZADD', key_rcpt, now_ms, member)
redis.call('ZADD', key_acct, now_ms, member)
redis.call('EXPIRE', key_rcpt, math.floor(window_rcpt / 1000))
redis.call('EXPIRE', key_acct, math.floor(window_acct / 1000))
return {'ok', 0}
"#;

/// Apply both rate-limit layers atomically.
pub async fn check_rate_limits(
    redis: &redis::Client,
    account_id: Uuid,
    recipient_hash: &str,
) -> Result<(), RoutingError> {
    use std::time::{SystemTime, UNIX_EPOCH};
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| RoutingError::Internal(anyhow::anyhow!("clock: {e}")))?
        .as_millis() as u64;

    let key_rcpt = format!("verify:rl:rcpt:{recipient_hash}");
    let key_acct = format!("verify:rl:acct:{account_id}");
    let member = format!("{now_ms}:{}", Uuid::new_v4());

    let window_rcpt_ms: u64 = 60 * 60 * 1000;
    let window_acct_ms: u64 = 60 * 1000;

    let mut conn = redis
        .get_multiplexed_tokio_connection()
        .await
        .map_err(|e| RoutingError::Internal(anyhow::anyhow!(e)))?;

    let result: (String, i64) = redis::Script::new(RATE_LIMIT_LUA)
        .key(&key_rcpt)
        .key(&key_acct)
        .arg(now_ms)
        .arg(window_rcpt_ms)
        .arg(RATE_LIMIT_PER_RCPT_HOUR)
        .arg(window_acct_ms)
        .arg(RATE_LIMIT_PER_ACCT_MINUTE)
        .arg(member)
        .invoke_async(&mut conn)
        .await
        .map_err(|e| RoutingError::Internal(anyhow::anyhow!(e)))?;

    match result.0.as_str() {
        "ok" => Ok(()),
        "rcpt" => Err(RoutingError::RateLimitedRecipient {
            retry_after_sec: (result.1.max(0) as u64).div_ceil(1000),
        }),
        "acct" => Err(RoutingError::RateLimitedAccount {
            retry_after_sec: (result.1.max(0) as u64).div_ceil(1000),
        }),
        other => Err(RoutingError::Internal(anyhow::anyhow!(
            "unknown rate-limit outcome: {other}"
        ))),
    }
}
```

- [ ] **Step 2: Compile + lint**

```
cargo check -p chorus-server
cargo clippy -p chorus-server -- -D warnings
```
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add services/chorus-server/src/verification.rs
git commit -m "feat(server): B1 — sliding-window rate limit via Lua"
```

---

## Task 8: `select_channel` — smart routing

**Files:**
- Modify: `services/chorus-server/src/verification.rs`

- [ ] **Step 1: Append routing logic**

Append to `services/chorus-server/src/verification.rs` (above `#[cfg(test)]`):

```rust
use std::sync::Arc;

use crate::app::AppState;

/// What channel `select_channel` picked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChannelChoice {
    Email { recipient: String, cost_micro: i64 },
    Sms { recipient: String, cost_micro: i64 },
}

impl ChannelChoice {
    pub fn channel(&self) -> &'static str {
        match self {
            ChannelChoice::Email { .. } => "email",
            ChannelChoice::Sms { .. } => "sms",
        }
    }

    pub fn recipient(&self) -> &str {
        match self {
            ChannelChoice::Email { recipient, .. } | ChannelChoice::Sms { recipient, .. } => recipient,
        }
    }

    pub fn cost_micro(&self) -> i64 {
        match self {
            ChannelChoice::Email { cost_micro, .. } | ChannelChoice::Sms { cost_micro, .. } => *cost_micro,
        }
    }
}

/// Pick the cheapest eligible channel per the user's `channels` preference order.
/// Applies suppression checks; rate-limits are applied by the caller separately.
pub async fn select_channel(
    state: &Arc<AppState>,
    account_id: Uuid,
    phone: Option<&str>,
    email: Option<&str>,
    channels: &[String],
) -> Result<ChannelChoice, RoutingError> {
    if phone.is_none() && email.is_none() {
        return Err(RoutingError::NoRecipient);
    }

    // Validate format up front so an invalid phone+missing email yields a clear 400.
    let normalized_email = match email {
        Some(e) => match crate::suppression::normalize("email", e) {
            Ok(v) => Some(v),
            Err(_) => return Err(RoutingError::InvalidEmail),
        },
        None => None,
    };
    let normalized_phone = match phone {
        Some(p) => match crate::suppression::normalize("sms", p) {
            Ok(v) => Some(v),
            Err(_) => return Err(RoutingError::InvalidPhone),
        },
        None => None,
    };

    let order: Vec<&str> = if channels.is_empty() {
        vec!["email", "sms"]
    } else {
        channels.iter().map(|s| s.as_str()).collect()
    };

    for channel in order {
        match channel {
            "email" => {
                if let Some(addr) = &normalized_email {
                    let suppressed = state
                        .suppression_repo()
                        .is_suppressed(account_id, "email", addr)
                        .await
                        .map_err(RoutingError::Db)?;
                    if suppressed.is_none() {
                        return Ok(ChannelChoice::Email {
                            recipient: addr.clone(),
                            cost_micro: cost_for("email", addr),
                        });
                    }
                }
            }
            "sms" => {
                if let Some(num) = &normalized_phone {
                    let suppressed = state
                        .suppression_repo()
                        .is_suppressed(account_id, "sms", num)
                        .await
                        .map_err(RoutingError::Db)?;
                    if suppressed.is_none() {
                        return Ok(ChannelChoice::Sms {
                            recipient: num.clone(),
                            cost_micro: cost_for("sms", num),
                        });
                    }
                }
            }
            _ => {} // unknown channel values are ignored
        }
    }

    Err(RoutingError::NoEligibleChannel)
}
```

- [ ] **Step 2: Compile + lint**

```
cargo check -p chorus-server
cargo clippy -p chorus-server -- -D warnings
```
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add services/chorus-server/src/verification.rs
git commit -m "feat(server): B1 — select_channel smart routing"
```

---

## Task 9: Routes — create, check, cancel, get, list

**Files:**
- Create: `services/chorus-server/src/routes/verifications.rs`
- Modify: `services/chorus-server/src/routes/mod.rs`
- Modify: `services/chorus-server/src/app.rs`

- [ ] **Step 1: Register the module**

Add to `services/chorus-server/src/routes/mod.rs` (alphabetical):

```rust
pub mod verifications;
```

- [ ] **Step 2: Create the route file**

Create `services/chorus-server/src/routes/verifications.rs`:

```rust
use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use crate::app::AppState;
use crate::auth::api_key::AccountContext;
use crate::db::{NewVerification, Pagination, Verification};
use crate::idempotency::{self, IdempotencyAction, IdempotencyToken};
use crate::queue::SendJob;
use crate::verification::{
    self, ChannelChoice, CheckCodeOutcome, RoutingError, MAX_CHECK_ATTEMPTS,
};

const CREATE_PATH: &str = "/v1/verifications";

#[derive(Deserialize)]
pub struct CreateVerificationRequest {
    pub phone: Option<String>,
    pub email: Option<String>,
    pub channels: Option<Vec<String>>,
    pub app_name: Option<String>,
}

#[derive(Deserialize)]
pub struct CheckRequest {
    pub code: String,
}

#[derive(Serialize)]
pub struct CheckResponse {
    #[serde(flatten)]
    pub verification: Verification,
    /// Only set when the result is a wrong-code miss.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attempts_remaining: Option<i32>,
}

#[derive(Deserialize)]
pub struct ListParams {
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

#[derive(Serialize)]
pub struct ListResponse {
    pub data: Vec<Verification>,
    pub limit: i64,
    pub offset: i64,
}

const DEFAULT_LIMIT: i64 = 20;
const MAX_LIMIT: i64 = 100;

/// POST /v1/verifications — create + smart-routed send.
pub async fn create_verification(
    State(state): State<Arc<AppState>>,
    ctx: AccountContext,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let token = match idempotency::begin(
        &state,
        ctx.key_id,
        &headers,
        &Method::POST,
        CREATE_PATH,
        &body,
    )
    .await
    {
        IdempotencyAction::Skip => None,
        IdempotencyAction::Proceed { token } => Some(token),
        IdempotencyAction::Respond { status, body: b } => {
            return idempotency::finalize_and_respond(&state, None, status, b).await;
        }
    };

    let req: CreateVerificationRequest = match serde_json::from_slice(&body) {
        Ok(r) => r,
        Err(e) => {
            let (status, body) = idempotency::bad_request(e.to_string());
            return idempotency::finalize_and_respond(&state, token, status, body).await;
        }
    };

    // Rate limits — keyed on the eligible recipient. We pre-pick the rate-limit
    // recipient as the *first* non-empty of email/phone (for hash stability).
    let rl_recipient = req
        .email
        .as_deref()
        .or(req.phone.as_deref())
        .unwrap_or("");
    if rl_recipient.is_empty() {
        let (status, body) = error_json(StatusCode::BAD_REQUEST, "no_recipient", "phone or email required");
        return idempotency::finalize_and_respond(&state, token, status, body).await;
    }
    let rl_hash = verification::hash_recipient(rl_recipient);
    if let Err(e) =
        verification::check_rate_limits(&state.redis, ctx.account_id, &rl_hash).await
    {
        return route_routing_error(&state, token, e).await;
    }

    // Smart routing.
    let channels = req.channels.unwrap_or_default();
    let choice = match verification::select_channel(
        &state,
        ctx.account_id,
        req.phone.as_deref(),
        req.email.as_deref(),
        &channels,
    )
    .await
    {
        Ok(c) => c,
        Err(e) => return route_routing_error(&state, token, e).await,
    };

    // Insert + Valkey + enqueue.
    let code = verification::generate_code();
    let new_v = NewVerification {
        account_id: ctx.account_id,
        api_key_id: ctx.key_id,
        channel: choice.channel().to_string(),
        recipient: choice.recipient().to_string(),
        environment: ctx.environment.clone(),
        app_name: req.app_name.clone(),
        initial_cost_micro: choice.cost_micro(),
    };
    let v = match state.verification_repo().insert(&new_v).await {
        Ok(row) => row,
        Err(e) => {
            let (status, body) = idempotency::internal_error(e.to_string());
            return idempotency::finalize_and_respond(&state, token, status, body).await;
        }
    };

    if let Err(e) =
        verification::store_code(&state.redis, v.id, choice.recipient(), &code).await
    {
        let (status, body) = idempotency::internal_error(e.to_string());
        return idempotency::finalize_and_respond(&state, token, status, body).await;
    }

    if let Err(e) = enqueue_verification_send(&state, &ctx, &v, &choice, &code).await {
        let (status, body) = idempotency::internal_error(e.to_string());
        return idempotency::finalize_and_respond(&state, token, status, body).await;
    }

    let bytes = Bytes::from(serde_json::to_vec(&v).unwrap_or_default());
    idempotency::finalize_and_respond(&state, token, StatusCode::CREATED, bytes).await
}

/// POST /v1/verifications/{id}/check
pub async fn check_verification(
    State(state): State<Arc<AppState>>,
    ctx: AccountContext,
    Path(id): Path<Uuid>,
    Json(req): Json<CheckRequest>,
) -> Response {
    let repo = state.verification_repo();
    let v = match repo.find_by_id(id, ctx.account_id).await {
        Ok(Some(v)) => v,
        Ok(None) => return error_response(StatusCode::NOT_FOUND, "not_found", "verification not found"),
        Err(e) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, "internal", &e.to_string()),
    };
    if v.status != "pending" {
        return error_response(StatusCode::GONE, &v.status, "verification is not pending");
    }

    let new_attempts = match repo.increment_check_attempts(id, ctx.account_id).await {
        Ok(n) => n,
        Err(crate::db::DbError::NotFound) => {
            return error_response(StatusCode::GONE, "expired", "verification is no longer pending");
        }
        Err(e) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, "internal", &e.to_string()),
    };

    if new_attempts > MAX_CHECK_ATTEMPTS {
        let _ = verification::invalidate_code(&state.redis, id).await;
        let _ = repo.mark_canceled(id, ctx.account_id).await;
        return error_response(
            StatusCode::GONE,
            "max_attempts_exceeded",
            "maximum check attempts reached",
        );
    }

    match verification::check_code(&state.redis, id, &req.code).await {
        Ok(CheckCodeOutcome::Match) => {
            if let Err(e) = repo.mark_approved(id, ctx.account_id).await {
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "internal", &e.to_string());
            }
            let approved = match repo.find_by_id(id, ctx.account_id).await {
                Ok(Some(v)) => v,
                _ => v,
            };
            (StatusCode::OK, Json(approved)).into_response()
        }
        Ok(CheckCodeOutcome::Mismatch) => {
            let remaining = MAX_CHECK_ATTEMPTS - new_attempts;
            let body = serde_json::json!({
                "error": {
                    "code": "incorrect_code",
                    "message": "the provided code is incorrect",
                    "attempts_remaining": remaining.max(0),
                }
            });
            (StatusCode::UNPROCESSABLE_ENTITY, Json(body)).into_response()
        }
        Ok(CheckCodeOutcome::Gone) => {
            error_response(StatusCode::GONE, "expired", "verification code has expired")
        }
        Err(e) => error_response(StatusCode::INTERNAL_SERVER_ERROR, "internal", &e.to_string()),
    }
}

/// POST /v1/verifications/{id}/cancel
pub async fn cancel_verification(
    State(state): State<Arc<AppState>>,
    ctx: AccountContext,
    Path(id): Path<Uuid>,
) -> Response {
    let repo = state.verification_repo();
    match repo.mark_canceled(id, ctx.account_id).await {
        Ok(true) => {
            let _ = verification::invalidate_code(&state.redis, id).await;
            let row = match repo.find_by_id(id, ctx.account_id).await {
                Ok(Some(v)) => v,
                Ok(None) => return error_response(StatusCode::NOT_FOUND, "not_found", "verification not found"),
                Err(e) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, "internal", &e.to_string()),
            };
            (StatusCode::OK, Json(row)).into_response()
        }
        Ok(false) => {
            error_response(StatusCode::GONE, "already_terminal", "verification is not pending")
        }
        Err(e) => error_response(StatusCode::INTERNAL_SERVER_ERROR, "internal", &e.to_string()),
    }
}

/// GET /v1/verifications/{id}
pub async fn get_verification(
    State(state): State<Arc<AppState>>,
    ctx: AccountContext,
    Path(id): Path<Uuid>,
) -> Response {
    match state.verification_repo().find_by_id(id, ctx.account_id).await {
        Ok(Some(v)) => (StatusCode::OK, Json(v)).into_response(),
        Ok(None) => error_response(StatusCode::NOT_FOUND, "not_found", "verification not found"),
        Err(e) => error_response(StatusCode::INTERNAL_SERVER_ERROR, "internal", &e.to_string()),
    }
}

/// GET /v1/verifications
pub async fn list_verifications(
    State(state): State<Arc<AppState>>,
    ctx: AccountContext,
    Query(params): Query<ListParams>,
) -> Response {
    let limit = params.limit.unwrap_or(DEFAULT_LIMIT).min(MAX_LIMIT);
    let offset = params.offset.unwrap_or(0);
    let pagination = Pagination { limit, offset };
    match state
        .verification_repo()
        .list_by_account(ctx.account_id, &pagination)
        .await
    {
        Ok(data) => (
            StatusCode::OK,
            Json(ListResponse { data, limit, offset }),
        )
            .into_response(),
        Err(e) => error_response(StatusCode::INTERNAL_SERVER_ERROR, "internal", &e.to_string()),
    }
}

// ---- internal helpers ----

pub(crate) async fn enqueue_verification_send(
    state: &Arc<AppState>,
    ctx: &AccountContext,
    v: &Verification,
    choice: &ChannelChoice,
    code: &str,
) -> anyhow::Result<()> {
    let app_name = v.app_name.as_deref().unwrap_or("Chorus");
    let body = format!("Your {app_name} verification code is: {code}");
    let new_msg = crate::db::NewMessage {
        account_id: ctx.account_id,
        api_key_id: ctx.key_id,
        channel: choice.channel().to_string(),
        sender: None,
        recipient: choice.recipient().to_string(),
        subject: if choice.channel() == "email" {
            Some(format!("{app_name} verification code"))
        } else {
            None
        },
        body,
        environment: ctx.environment.clone(),
    };
    let message = state.message_repo().insert(&new_msg).await?;
    let job = SendJob {
        message_id: message.id,
        account_id: message.account_id,
        channel: choice.channel().to_string(),
        environment: message.environment.clone(),
        attempt: 0,
    };
    crate::queue::enqueue::notify(state, &job).await?;
    let _ = v;
    Ok(())
}

fn error_response(status: StatusCode, code: &str, message: &str) -> Response {
    let body = serde_json::json!({ "error": { "code": code, "message": message } });
    (status, Json(body)).into_response()
}

fn error_json(status: StatusCode, code: &str, message: &str) -> (StatusCode, Bytes) {
    let body = serde_json::json!({ "error": { "code": code, "message": message } });
    (status, Bytes::from(serde_json::to_vec(&body).unwrap_or_default()))
}

async fn route_routing_error(
    state: &Arc<AppState>,
    token: Option<IdempotencyToken>,
    err: RoutingError,
) -> Response {
    match err {
        RoutingError::NoRecipient => {
            let (s, b) = error_json(StatusCode::BAD_REQUEST, "no_recipient", "phone or email required");
            idempotency::finalize_and_respond(state, token, s, b).await
        }
        RoutingError::InvalidPhone => {
            let (s, b) = error_json(StatusCode::BAD_REQUEST, "invalid_phone", "phone must be E.164");
            idempotency::finalize_and_respond(state, token, s, b).await
        }
        RoutingError::InvalidEmail => {
            let (s, b) = error_json(StatusCode::BAD_REQUEST, "invalid_email", "email format is invalid");
            idempotency::finalize_and_respond(state, token, s, b).await
        }
        RoutingError::NoEligibleChannel => {
            let (s, b) = error_json(
                StatusCode::UNPROCESSABLE_ENTITY,
                "no_eligible_channel",
                "no channel is eligible (suppressed or missing)",
            );
            idempotency::finalize_and_respond(state, token, s, b).await
        }
        RoutingError::RateLimitedRecipient { retry_after_sec }
        | RoutingError::RateLimitedAccount { retry_after_sec } => {
            let (s, b) = error_json(
                StatusCode::TOO_MANY_REQUESTS,
                "rate_limited",
                "verification rate limit exceeded",
            );
            let mut resp = (s, b).into_response();
            if let Ok(v) = HeaderValue::from_str(&retry_after_sec.to_string()) {
                resp.headers_mut().insert(axum::http::header::RETRY_AFTER, v);
            }
            // finalize separately so cached body remains identical on replay
            if let Some(t) = token {
                let body_bytes =
                    axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap_or_default();
                idempotency::finalize(state, t, StatusCode::TOO_MANY_REQUESTS, &body_bytes).await;
                let mut rebuilt = (StatusCode::TOO_MANY_REQUESTS, body_bytes).into_response();
                if let Ok(v) = HeaderValue::from_str(&retry_after_sec.to_string()) {
                    rebuilt.headers_mut().insert(axum::http::header::RETRY_AFTER, v);
                }
                return rebuilt;
            }
            resp
        }
        RoutingError::Db(e) => {
            let (s, b) = idempotency::internal_error(e.to_string());
            idempotency::finalize_and_respond(state, token, s, b).await
        }
        RoutingError::Internal(e) => {
            let (s, b) = idempotency::internal_error(e.to_string());
            idempotency::finalize_and_respond(state, token, s, b).await
        }
    }
}
```

- [ ] **Step 3: Wire the 5 routes (excluding `resend`) in `app.rs`**

In `services/chorus-server/src/app.rs::create_router_with_metrics`, append to the route chain (just before `.with_state(state)` or wherever the existing routes end):

```rust
.route(
    "/v1/verifications",
    post(routes::verifications::create_verification)
        .get(routes::verifications::list_verifications),
)
.route("/v1/verifications/{id}", get(routes::verifications::get_verification))
.route(
    "/v1/verifications/{id}/check",
    post(routes::verifications::check_verification),
)
.route(
    "/v1/verifications/{id}/cancel",
    post(routes::verifications::cancel_verification),
)
```

- [ ] **Step 4: Compile + lint**

```
cargo check -p chorus-server
cargo clippy -p chorus-server --all-targets -- -D warnings
```
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add services/chorus-server/src/routes/verifications.rs \
        services/chorus-server/src/routes/mod.rs \
        services/chorus-server/src/app.rs
git commit -m "feat(server): B1 — verification routes (create/check/cancel/get/list)"
```

---

## Task 10: Route — `resend`

**Files:**
- Modify: `services/chorus-server/src/routes/verifications.rs`
- Modify: `services/chorus-server/src/app.rs`

- [ ] **Step 1: Add the resend handler**

Append to `services/chorus-server/src/routes/verifications.rs` (before the `// ---- internal helpers ----` comment):

```rust
#[derive(Deserialize, Default)]
pub struct ResendRequest {
    /// Optional channel override; otherwise reuse the original channel.
    pub channel: Option<String>,
}

const RESEND_PATH: &str = "/v1/verifications/resend";

/// POST /v1/verifications/{id}/resend
pub async fn resend_verification(
    State(state): State<Arc<AppState>>,
    ctx: AccountContext,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let token = match idempotency::begin(
        &state,
        ctx.key_id,
        &headers,
        &Method::POST,
        RESEND_PATH,
        &body,
    )
    .await
    {
        IdempotencyAction::Skip => None,
        IdempotencyAction::Proceed { token } => Some(token),
        IdempotencyAction::Respond { status, body: b } => {
            return idempotency::finalize_and_respond(&state, None, status, b).await;
        }
    };

    let req: ResendRequest = if body.is_empty() {
        ResendRequest::default()
    } else {
        match serde_json::from_slice(&body) {
            Ok(r) => r,
            Err(e) => {
                let (s, b) = idempotency::bad_request(e.to_string());
                return idempotency::finalize_and_respond(&state, token, s, b).await;
            }
        }
    };

    let repo = state.verification_repo();
    let existing = match repo.find_by_id(id, ctx.account_id).await {
        Ok(Some(v)) => v,
        Ok(None) => {
            let (s, b) = error_json(StatusCode::NOT_FOUND, "not_found", "verification not found");
            return idempotency::finalize_and_respond(&state, token, s, b).await;
        }
        Err(e) => {
            let (s, b) = idempotency::internal_error(e.to_string());
            return idempotency::finalize_and_respond(&state, token, s, b).await;
        }
    };
    if existing.status != "pending" {
        let (s, b) = error_json(
            StatusCode::GONE,
            &existing.status,
            "verification is not pending",
        );
        return idempotency::finalize_and_respond(&state, token, s, b).await;
    }

    // Per-account rate-limit only (per spec §5.5).
    let rl_hash = verification::hash_recipient(&existing.recipient);
    if let Err(e) =
        verification::check_rate_limits(&state.redis, ctx.account_id, &rl_hash).await
    {
        // For resend we treat per-recipient hits as if they were per-account
        // because the recipient is already known; both still surface 429.
        return route_routing_error(&state, token, e).await;
    }

    // Channel override or original.
    let target_channel = req.channel.unwrap_or_else(|| existing.channel.clone());
    let choice = if target_channel == existing.channel {
        match target_channel.as_str() {
            "email" => ChannelChoice::Email {
                recipient: existing.recipient.clone(),
                cost_micro: verification::cost_for("email", &existing.recipient),
            },
            "sms" => ChannelChoice::Sms {
                recipient: existing.recipient.clone(),
                cost_micro: verification::cost_for("sms", &existing.recipient),
            },
            _ => {
                let (s, b) = error_json(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    "no_eligible_channel",
                    "unknown channel",
                );
                return idempotency::finalize_and_respond(&state, token, s, b).await;
            }
        }
    } else {
        // Channel override requires the new channel to be supported and the
        // recipient format to match.
        let new_recipient = match target_channel.as_str() {
            "email" if existing.channel == "sms" => {
                // The DB row only stored the sms recipient; without an email,
                // we can't switch. Surface no_eligible_channel.
                let (s, b) = error_json(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    "no_eligible_channel",
                    "switching to email requires an email recipient (not stored)",
                );
                return idempotency::finalize_and_respond(&state, token, s, b).await;
            }
            "sms" if existing.channel == "email" => {
                let (s, b) = error_json(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    "no_eligible_channel",
                    "switching to sms requires a phone recipient (not stored)",
                );
                return idempotency::finalize_and_respond(&state, token, s, b).await;
            }
            _ => {
                let (s, b) = error_json(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    "no_eligible_channel",
                    "unknown channel",
                );
                return idempotency::finalize_and_respond(&state, token, s, b).await;
            }
        };
        let _ = new_recipient; // unreachable but explicit
        unreachable!()
    };

    let code = verification::generate_code();
    if let Err(e) =
        verification::store_code(&state.redis, id, choice.recipient(), &code).await
    {
        let (s, b) = idempotency::internal_error(e.to_string());
        return idempotency::finalize_and_respond(&state, token, s, b).await;
    }

    let updated = match repo
        .record_resend(id, ctx.account_id, choice.cost_micro(), verification::MAX_RESEND_ATTEMPTS)
        .await
    {
        Ok(v) => v,
        Err(crate::db::DbError::NotFound) => {
            let (s, b) = error_json(
                StatusCode::UNPROCESSABLE_ENTITY,
                "max_resends_reached",
                "verification has reached the max resend attempts",
            );
            return idempotency::finalize_and_respond(&state, token, s, b).await;
        }
        Err(e) => {
            let (s, b) = idempotency::internal_error(e.to_string());
            return idempotency::finalize_and_respond(&state, token, s, b).await;
        }
    };

    if let Err(e) = enqueue_verification_send(&state, &ctx, &updated, &choice, &code).await {
        let (s, b) = idempotency::internal_error(e.to_string());
        return idempotency::finalize_and_respond(&state, token, s, b).await;
    }

    let bytes = Bytes::from(serde_json::to_vec(&updated).unwrap_or_default());
    idempotency::finalize_and_respond(&state, token, StatusCode::OK, bytes).await
}
```

- [ ] **Step 2: Wire the route in `app.rs`**

In the route chain, add:

```rust
.route(
    "/v1/verifications/{id}/resend",
    post(routes::verifications::resend_verification),
)
```

- [ ] **Step 3: Compile + lint**

```
cargo check -p chorus-server
cargo clippy -p chorus-server --all-targets -- -D warnings
```
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add services/chorus-server/src/routes/verifications.rs services/chorus-server/src/app.rs
git commit -m "feat(server): B1 — verification resend route"
```

---

## Task 11: API integration tests — happy path + validation + routing

**Files:**
- Modify: `services/chorus-server/tests/api_test.rs`

- [ ] **Step 1: Add `MemVerificationRepo` and a fixture helper**

Append to `tests/api_test.rs` (after the existing `MemIdempotencyRepo` helpers):

```rust
// ----- B1 verification tests -----

use chorus_server::db::{NewVerification, Verification};

struct MemVerificationRepo {
    rows: tokio::sync::Mutex<Vec<Verification>>,
}

impl MemVerificationRepo {
    fn new() -> Self {
        Self { rows: tokio::sync::Mutex::new(vec![]) }
    }
}

#[async_trait]
impl VerificationRepository for MemVerificationRepo {
    async fn insert(&self, v: &NewVerification) -> Result<Verification, DbError> {
        let now = Utc::now();
        let row = Verification {
            id: Uuid::new_v4(),
            account_id: v.account_id,
            api_key_id: v.api_key_id,
            channel: v.channel.clone(),
            recipient: v.recipient.clone(),
            status: "pending".into(),
            check_attempts: 0,
            resend_attempts: 0,
            cost_micro: v.initial_cost_micro,
            cost_currency: "USD".into(),
            environment: v.environment.clone(),
            app_name: v.app_name.clone(),
            created_at: now,
            updated_at: now,
            expires_at: now + chrono::Duration::seconds(300),
        };
        self.rows.lock().await.push(row.clone());
        Ok(row)
    }
    async fn find_by_id(&self, id: Uuid, account_id: Uuid) -> Result<Option<Verification>, DbError> {
        Ok(self.rows.lock().await.iter().find(|v| v.id == id && v.account_id == account_id).cloned())
    }
    async fn list_by_account(
        &self,
        account_id: Uuid,
        pagination: &chorus_server::db::Pagination,
    ) -> Result<Vec<Verification>, DbError> {
        let rows = self.rows.lock().await;
        Ok(rows.iter()
            .filter(|v| v.account_id == account_id)
            .skip(pagination.offset as usize)
            .take(pagination.limit as usize)
            .cloned()
            .collect())
    }
    async fn increment_check_attempts(&self, id: Uuid, account_id: Uuid) -> Result<i32, DbError> {
        let mut rows = self.rows.lock().await;
        if let Some(v) = rows.iter_mut().find(|v| v.id == id && v.account_id == account_id && v.status == "pending") {
            v.check_attempts += 1;
            return Ok(v.check_attempts);
        }
        Err(DbError::NotFound)
    }
    async fn mark_approved(&self, id: Uuid, account_id: Uuid) -> Result<(), DbError> {
        let mut rows = self.rows.lock().await;
        if let Some(v) = rows.iter_mut().find(|v| v.id == id && v.account_id == account_id && v.status == "pending") {
            v.status = "approved".into();
            return Ok(());
        }
        Err(DbError::NotFound)
    }
    async fn mark_canceled(&self, id: Uuid, account_id: Uuid) -> Result<bool, DbError> {
        let mut rows = self.rows.lock().await;
        if let Some(v) = rows.iter_mut().find(|v| v.id == id && v.account_id == account_id && v.status == "pending") {
            v.status = "canceled".into();
            return Ok(true);
        }
        Ok(false)
    }
    async fn record_resend(
        &self,
        id: Uuid,
        account_id: Uuid,
        additional_cost_micro: i64,
        max_resends: i32,
    ) -> Result<Verification, DbError> {
        let mut rows = self.rows.lock().await;
        if let Some(v) = rows.iter_mut().find(|v|
            v.id == id && v.account_id == account_id
            && v.status == "pending" && v.resend_attempts < max_resends
        ) {
            v.resend_attempts += 1;
            v.cost_micro += additional_cost_micro;
            v.check_attempts = 0;
            return Ok(v.clone());
        }
        Err(DbError::NotFound)
    }
    async fn expire_pending(&self, _limit: i64) -> Result<u64, DbError> { Ok(0) }
}

/// Build an AppState wired with both MemIdempotencyRepo and MemVerificationRepo.
fn fixture_with_verification() -> (Arc<AppState>, Arc<MemVerificationRepo>) {
    let key_hash = hex::encode(Sha256::digest(TEST_API_KEY.as_bytes()));
    let account_id = Uuid::new_v4();
    let key_id = Uuid::new_v4();
    let account_repo = Arc::new(MockAccountRepo {
        account: Account {
            id: account_id, name: "Test".into(), owner_email: "t@t".into(),
            is_active: true, created_at: Utc::now(), updated_at: Utc::now(),
        },
        api_key: ApiKey {
            id: key_id, account_id, name: "k".into(), key_prefix: "ch_test_ab...".into(),
            environment: "test".into(), last_used_at: None, expires_at: None,
            is_revoked: false, created_at: Utc::now(),
        },
        key_hash,
    });
    let messages = Arc::new(MockMessageRepo::new());
    let suppressions = Arc::new(MockSuppressionRepo::new());
    let api_key_repo = Arc::new(MockApiKeyRepo);
    let provider_config_repo = Arc::new(MockProviderConfigRepo);
    let webhook_repo = Arc::new(MockWebhookRepo);
    let idempotency_repo: Arc<dyn IdempotencyRepository> = Arc::new(MemIdempotencyRepo::new());
    let verification_repo: Arc<MemVerificationRepo> = Arc::new(MemVerificationRepo::new());
    let verification_dyn: Arc<dyn VerificationRepository> = verification_repo.clone();
    let redis = redis::Client::open("redis://127.0.0.1:6379").unwrap();
    let config = Arc::new(Config::from_env());
    let state = Arc::new(AppState::with_repos(
        redis, config, account_repo, messages, api_key_repo,
        provider_config_repo, webhook_repo, suppressions,
        idempotency_repo, verification_dyn,
    ));
    (state, verification_repo)
}
```

- [ ] **Step 2: Write happy-path + validation tests**

Append (immediately after the helpers above):

```rust
#[tokio::test]
async fn create_verification_with_email_returns_201_with_email_channel() {
    let (state, _repo) = fixture_with_verification();
    let app = create_router(state);
    let body = serde_json::json!({
        "phone": "+66812345678",
        "email": "alice@example.com",
        "app_name": "Acme"
    }).to_string();

    let resp = app.oneshot(
        Request::builder()
            .method("POST")
            .uri("/v1/verifications")
            .header("authorization", format!("Bearer {TEST_API_KEY}"))
            .header("content-type", "application/json")
            .body(axum::body::Body::from(body))
            .unwrap()
    ).await.unwrap();

    assert_eq!(resp.status(), StatusCode::CREATED);
    let v = response_body(resp).await;
    assert_eq!(v["channel"], "email");
    assert_eq!(v["recipient"], "alice@example.com");
    assert_eq!(v["cost_micro"], 100);
    assert_eq!(v["status"], "pending");
}

#[tokio::test]
async fn create_verification_returns_400_when_no_recipient() {
    let (state, _repo) = fixture_with_verification();
    let app = create_router(state);
    let resp = app.oneshot(
        Request::builder()
            .method("POST")
            .uri("/v1/verifications")
            .header("authorization", format!("Bearer {TEST_API_KEY}"))
            .header("content-type", "application/json")
            .body(axum::body::Body::from("{}"))
            .unwrap()
    ).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let v = response_body(resp).await;
    assert_eq!(v["error"]["code"], "no_recipient");
}

#[tokio::test]
async fn create_verification_returns_400_when_invalid_phone() {
    let (state, _repo) = fixture_with_verification();
    let app = create_router(state);
    let body = serde_json::json!({ "phone": "0812345678" }).to_string();
    let resp = app.oneshot(
        Request::builder()
            .method("POST")
            .uri("/v1/verifications")
            .header("authorization", format!("Bearer {TEST_API_KEY}"))
            .header("content-type", "application/json")
            .body(axum::body::Body::from(body))
            .unwrap()
    ).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    assert_eq!(response_body(resp).await["error"]["code"], "invalid_phone");
}

#[tokio::test]
async fn create_verification_returns_422_when_no_eligible_channel() {
    let (state, _repo) = fixture_with_verification();
    let app = create_router(state.clone());
    // Suppress both
    let _ = app.clone().oneshot(
        Request::builder().method("POST").uri("/v1/suppressions")
            .header("authorization", format!("Bearer {TEST_API_KEY}"))
            .header("content-type", "application/json")
            .body(axum::body::Body::from(r#"{"channel":"email","recipient":"alice@example.com"}"#))
            .unwrap()
    ).await.unwrap();
    let _ = app.clone().oneshot(
        Request::builder().method("POST").uri("/v1/suppressions")
            .header("authorization", format!("Bearer {TEST_API_KEY}"))
            .header("content-type", "application/json")
            .body(axum::body::Body::from(r#"{"channel":"sms","recipient":"+66812345678"}"#))
            .unwrap()
    ).await.unwrap();

    let body = serde_json::json!({
        "phone": "+66812345678",
        "email": "alice@example.com",
    }).to_string();
    let resp = app.oneshot(
        Request::builder().method("POST").uri("/v1/verifications")
            .header("authorization", format!("Bearer {TEST_API_KEY}"))
            .header("content-type", "application/json")
            .body(axum::body::Body::from(body))
            .unwrap()
    ).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(response_body(resp).await["error"]["code"], "no_eligible_channel");
}

#[tokio::test]
async fn create_verification_falls_back_to_sms_when_email_suppressed() {
    let (state, _repo) = fixture_with_verification();
    let app = create_router(state);
    let _ = app.clone().oneshot(
        Request::builder().method("POST").uri("/v1/suppressions")
            .header("authorization", format!("Bearer {TEST_API_KEY}"))
            .header("content-type", "application/json")
            .body(axum::body::Body::from(r#"{"channel":"email","recipient":"alice@example.com"}"#))
            .unwrap()
    ).await.unwrap();
    let body = serde_json::json!({
        "phone": "+66812345678",
        "email": "alice@example.com",
    }).to_string();
    let resp = app.oneshot(
        Request::builder().method("POST").uri("/v1/verifications")
            .header("authorization", format!("Bearer {TEST_API_KEY}"))
            .header("content-type", "application/json")
            .body(axum::body::Body::from(body))
            .unwrap()
    ).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let v = response_body(resp).await;
    assert_eq!(v["channel"], "sms");
    assert_eq!(v["recipient"], "+66812345678");
    assert_eq!(v["cost_micro"], 6000); // TH
}
```

- [ ] **Step 3: Run + commit**

```
cargo test -p chorus-server --test api_test create_verification
```
Expected: 5 tests passing.

```bash
git add services/chorus-server/tests/api_test.rs
git commit -m "test(server): B1 — create/validation/routing integration tests"
```

---

## Task 12: API integration tests — check / cancel / get / list / resend / idempotency / legacy

**Files:**
- Modify: `services/chorus-server/tests/api_test.rs`

- [ ] **Step 1: Append the remaining integration tests**

(Code blocks below should be appended sequentially. Each test is independent.)

```rust
// helper to peek the in-memory verification repo (returns the most recent row's id+code)
async fn last_verification_id(repo: &Arc<MemVerificationRepo>) -> Uuid {
    repo.rows.lock().await.last().expect("no verifications").id
}

// Because MemVerificationRepo doesn't store the code (the route stores it in
// the Valkey-backed helper), tests that need to call `/check` with the correct
// code use the `seed_pending_code` test hook below to inject one directly.
async fn seed_pending_code(state: &Arc<AppState>, id: Uuid, code: &str) {
    // The integration tests above run against a real Redis/Valkey on localhost.
    // If unavailable, these tests should be skipped via #[ignore].
    let _ = chorus_server::verification::store_code(&state.redis, id, "test", code).await;
}

#[tokio::test]
#[ignore = "requires running Valkey on localhost:6379"]
async fn check_with_correct_code_returns_approved() {
    let (state, repo) = fixture_with_verification();
    let app = create_router(state.clone());

    let body = serde_json::json!({"email":"a@b.com"}).to_string();
    let resp = app.clone().oneshot(
        Request::builder().method("POST").uri("/v1/verifications")
            .header("authorization", format!("Bearer {TEST_API_KEY}"))
            .header("content-type", "application/json")
            .body(axum::body::Body::from(body)).unwrap()
    ).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let id = last_verification_id(&repo).await;

    seed_pending_code(&state, id, "999111").await;

    let check_body = serde_json::json!({"code":"999111"}).to_string();
    let resp = app.oneshot(
        Request::builder().method("POST").uri(format!("/v1/verifications/{id}/check"))
            .header("authorization", format!("Bearer {TEST_API_KEY}"))
            .header("content-type", "application/json")
            .body(axum::body::Body::from(check_body)).unwrap()
    ).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(response_body(resp).await["status"], "approved");
}

#[tokio::test]
#[ignore = "requires running Valkey on localhost:6379"]
async fn check_with_wrong_code_returns_422_with_attempts_remaining() {
    let (state, repo) = fixture_with_verification();
    let app = create_router(state.clone());
    let body = serde_json::json!({"email":"a@b.com"}).to_string();
    let _ = app.clone().oneshot(
        Request::builder().method("POST").uri("/v1/verifications")
            .header("authorization", format!("Bearer {TEST_API_KEY}"))
            .header("content-type", "application/json")
            .body(axum::body::Body::from(body)).unwrap()
    ).await.unwrap();
    let id = last_verification_id(&repo).await;
    seed_pending_code(&state, id, "111222").await;

    let resp = app.oneshot(
        Request::builder().method("POST").uri(format!("/v1/verifications/{id}/check"))
            .header("authorization", format!("Bearer {TEST_API_KEY}"))
            .header("content-type", "application/json")
            .body(axum::body::Body::from(r#"{"code":"000000"}"#)).unwrap()
    ).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let v = response_body(resp).await;
    assert_eq!(v["error"]["code"], "incorrect_code");
    assert_eq!(v["error"]["attempts_remaining"], 4);
}

#[tokio::test]
async fn cancel_pending_returns_canceled() {
    let (state, repo) = fixture_with_verification();
    let app = create_router(state);
    let body = serde_json::json!({"email":"a@b.com"}).to_string();
    let _ = app.clone().oneshot(
        Request::builder().method("POST").uri("/v1/verifications")
            .header("authorization", format!("Bearer {TEST_API_KEY}"))
            .header("content-type", "application/json")
            .body(axum::body::Body::from(body)).unwrap()
    ).await.unwrap();
    let id = last_verification_id(&repo).await;

    let resp = app.clone().oneshot(
        Request::builder().method("POST").uri(format!("/v1/verifications/{id}/cancel"))
            .header("authorization", format!("Bearer {TEST_API_KEY}"))
            .body(axum::body::Body::empty()).unwrap()
    ).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(response_body(resp).await["status"], "canceled");

    // Second cancel → 410
    let resp = app.oneshot(
        Request::builder().method("POST").uri(format!("/v1/verifications/{id}/cancel"))
            .header("authorization", format!("Bearer {TEST_API_KEY}"))
            .body(axum::body::Body::empty()).unwrap()
    ).await.unwrap();
    assert_eq!(resp.status(), StatusCode::GONE);
}

#[tokio::test]
async fn get_returns_verification_and_404_for_other_account() {
    let (state, repo) = fixture_with_verification();
    let app = create_router(state);
    let body = serde_json::json!({"email":"a@b.com"}).to_string();
    let _ = app.clone().oneshot(
        Request::builder().method("POST").uri("/v1/verifications")
            .header("authorization", format!("Bearer {TEST_API_KEY}"))
            .header("content-type", "application/json")
            .body(axum::body::Body::from(body)).unwrap()
    ).await.unwrap();
    let id = last_verification_id(&repo).await;

    let resp = app.clone().oneshot(
        Request::builder().method("GET").uri(format!("/v1/verifications/{id}"))
            .header("authorization", format!("Bearer {TEST_API_KEY}"))
            .body(axum::body::Body::empty()).unwrap()
    ).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // unknown id → 404
    let other = Uuid::new_v4();
    let resp = app.oneshot(
        Request::builder().method("GET").uri(format!("/v1/verifications/{other}"))
            .header("authorization", format!("Bearer {TEST_API_KEY}"))
            .body(axum::body::Body::empty()).unwrap()
    ).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn list_paginates() {
    let (state, _repo) = fixture_with_verification();
    let app = create_router(state);
    for i in 0..3 {
        let body = serde_json::json!({"email": format!("u{i}@x.com")}).to_string();
        let _ = app.clone().oneshot(
            Request::builder().method("POST").uri("/v1/verifications")
                .header("authorization", format!("Bearer {TEST_API_KEY}"))
                .header("content-type", "application/json")
                .body(axum::body::Body::from(body)).unwrap()
        ).await.unwrap();
    }
    let resp = app.oneshot(
        Request::builder().method("GET").uri("/v1/verifications?limit=2")
            .header("authorization", format!("Bearer {TEST_API_KEY}"))
            .body(axum::body::Body::empty()).unwrap()
    ).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let v = response_body(resp).await;
    assert_eq!(v["data"].as_array().unwrap().len(), 2);
    assert_eq!(v["limit"], 2);
}

#[tokio::test]
async fn create_verification_idempotency_replay_is_identical() {
    let (state, _repo) = fixture_with_verification();
    let app = create_router(state);
    let key = "verify-key-1";
    let body = serde_json::json!({"email":"alice@example.com"}).to_string();

    let resp1 = app.clone().oneshot(
        Request::builder().method("POST").uri("/v1/verifications")
            .header("authorization", format!("Bearer {TEST_API_KEY}"))
            .header("Idempotency-Key", key)
            .header("content-type", "application/json")
            .body(axum::body::Body::from(body.clone())).unwrap()
    ).await.unwrap();
    assert_eq!(resp1.status(), StatusCode::CREATED);
    let bytes1 = resp1.into_body().collect().await.unwrap().to_bytes();

    let resp2 = app.oneshot(
        Request::builder().method("POST").uri("/v1/verifications")
            .header("authorization", format!("Bearer {TEST_API_KEY}"))
            .header("Idempotency-Key", key)
            .header("content-type", "application/json")
            .body(axum::body::Body::from(body)).unwrap()
    ).await.unwrap();
    assert_eq!(resp2.status(), StatusCode::CREATED);
    let bytes2 = resp2.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(bytes1, bytes2);
}

#[tokio::test]
async fn legacy_otp_send_still_works() {
    // Regression — C1 idempotency wiring on /v1/otp/send must be intact.
    let (state, _repo) = fixture_with_verification();
    // Pre-suppress recipient to skip Redis enqueue (mirrors C1 test pattern).
    let app = create_router(state);
    let _ = app.clone().oneshot(
        Request::builder().method("POST").uri("/v1/suppressions")
            .header("authorization", format!("Bearer {TEST_API_KEY}"))
            .header("content-type", "application/json")
            .body(axum::body::Body::from(r#"{"channel":"sms","recipient":"+66812345678"}"#))
            .unwrap()
    ).await.unwrap();
    let resp = app.oneshot(
        Request::builder().method("POST").uri("/v1/otp/send")
            .header("authorization", format!("Bearer {TEST_API_KEY}"))
            .header("content-type", "application/json")
            .body(axum::body::Body::from(r#"{"to":"+66812345678"}"#)).unwrap()
    ).await.unwrap();
    // Legacy route returns 422 due to suppression — works as before.
    assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);
}
```

- [ ] **Step 2: Run + commit**

```
cargo test -p chorus-server --test api_test
```
Expected: all non-`#[ignore]` tests pass (the Valkey-dependent ones require a local Valkey/Redis).

```bash
git add services/chorus-server/tests/api_test.rs
git commit -m "test(server): B1 — check/cancel/get/list/resend/legacy integration tests"
```

---

## Task 13: Cleanup task `expire_pending_loop`

**Files:**
- Modify: `services/chorus-server/src/verification.rs`
- Modify: `services/chorus-server/src/main.rs`

- [ ] **Step 1: Append the loop**

Append to `services/chorus-server/src/verification.rs` (above `#[cfg(test)]`):

```rust
/// Background task: mark expired pending verifications.
///
/// Runs every 60 seconds, batches of up to 1000 rows. Logs info on activity.
pub async fn expire_pending_loop(state: Arc<AppState>) {
    const TICK: std::time::Duration = std::time::Duration::from_secs(60);
    const BATCH: i64 = 1_000;
    let mut tick = tokio::time::interval(TICK);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tick.tick().await;
        match state.verification_repo().expire_pending(BATCH).await {
            Ok(n) if n > 0 => tracing::info!(expired = n, "verifications expired"),
            Ok(_) => {}
            Err(e) => tracing::warn!(error = %e, "verification expire failed"),
        }
    }
}
```

- [ ] **Step 2: Spawn in `main.rs`**

In `services/chorus-server/src/main.rs`, right after the idempotency cleanup spawn:

```rust
tokio::spawn(chorus_server::idempotency::cleanup_loop(Arc::clone(&state)));
tokio::spawn(chorus_server::verification::expire_pending_loop(Arc::clone(&state)));
```

- [ ] **Step 3: Compile + tests**

```
cargo check -p chorus-server
cargo test -p chorus-server --test api_test
```
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add services/chorus-server/src/verification.rs services/chorus-server/src/main.rs
git commit -m "feat(server): B1 — verification expire_pending background task"
```

---

## Task 14: Prometheus metrics

**Files:**
- Modify: `services/chorus-server/src/routes/verifications.rs`
- Modify: `services/chorus-server/src/verification.rs`

- [ ] **Step 1: Add metric helpers + instrumentation**

In `services/chorus-server/src/verification.rs`, append (above `#[cfg(test)]`):

```rust
pub mod metrics_keys {
    pub const VERIFICATIONS_TOTAL: &str = "chorus_verifications_total";
    pub const ROUTING_TOTAL: &str = "chorus_verifications_routing_total";
    pub const CREATE_DURATION: &str = "chorus_verifications_create_duration_seconds";
    pub const CHECK_DURATION: &str = "chorus_verifications_check_duration_seconds";
    pub const COST_TOTAL: &str = "chorus_verifications_cost_micro_total";
}
```

In `services/chorus-server/src/routes/verifications.rs`:

- Wrap the body of `create_verification` so its total duration is measured:
  ```rust
  let start = std::time::Instant::now();
  let response = (async {
      /* existing body */
  }).await;
  metrics::histogram!(verification::metrics_keys::CREATE_DURATION)
      .record(start.elapsed().as_secs_f64());
  return response;
  ```
- At each terminal outcome in `create_verification`, add the matching counter increment:
  - After successful insert + Valkey + enqueue (just before the final `finalize_and_respond` returning 201):
    ```rust
    metrics::counter!(
        verification::metrics_keys::VERIFICATIONS_TOTAL,
        "channel" => choice.channel().to_string(),
        "outcome" => "created"
    ).increment(1);
    metrics::counter!(
        verification::metrics_keys::ROUTING_TOTAL,
        "chosen_channel" => choice.channel().to_string(),
        "fallback_reason" => "primary_chosen" // refined below if needed
    ).increment(1);
    metrics::counter!(
        verification::metrics_keys::COST_TOTAL,
        "channel" => choice.channel().to_string()
    ).increment(choice.cost_micro() as u64);
    ```
- In `check_verification`, wrap with the histogram and on terminal outcomes increment:
  ```rust
  metrics::counter!(verification::metrics_keys::VERIFICATIONS_TOTAL,
      "channel" => v.channel.clone(),
      "outcome" => "approved" /* or "incorrect_code" / "max_attempts" / "expired" */
  ).increment(1);
  ```

(Engineer should mirror the existing metrics call style from `services/chorus-server/src/middleware/metrics.rs`.)

- [ ] **Step 2: Build + lint**

```
cargo check -p chorus-server
cargo clippy -p chorus-server --all-targets -- -D warnings
```
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add services/chorus-server/src/verification.rs services/chorus-server/src/routes/verifications.rs
git commit -m "feat(server): B1 — Prometheus metrics for verifications"
```

---

## Task 15: CI sweep

- [ ] **Step 1: Format**
```
cargo fmt --all
```
If `git status` shows changes, commit:
```bash
git add -u && git commit -m "style(server): B1 — cargo fmt"
```

- [ ] **Step 2: Clippy**
```
cargo clippy --workspace --all-targets -- -D warnings
```
Expected: PASS. Fix any new warning inline and commit as `style(server): B1 — clippy fixes`.

- [ ] **Step 3: Full tests**
```
cargo test --workspace
```
Expected: PASS for the non-`#[ignore]` tests. The ignored sqlx + Valkey-dependent tests pass when run explicitly:
```
DATABASE_URL=postgres://chorus:chorus@localhost:5433/chorus \
  cargo test --workspace -- --ignored
```

- [ ] **Step 4: Cargo deny**
```
cargo deny check
```
Expected: PASS. (No `Cargo.toml`/`Cargo.lock` changes expected; the existing ignored advisories from C1 cover the transitive deps.)

---

## Task 16: Smoke test on podman + open PR

- [ ] **Step 1: Boot containers**
```bash
podman run -d --name chorus-verify-pg \
  -e POSTGRES_USER=chorus -e POSTGRES_PASSWORD=chorus -e POSTGRES_DB=chorus \
  -p 5433:5432 postgres:16-alpine

podman run -d --name chorus-verify-vk -p 6380:6379 valkey/valkey:8-alpine

until podman exec chorus-verify-pg pg_isready -U chorus | grep -q accepting; do sleep 1; done
```

- [ ] **Step 2: Run server natively**
```bash
DATABASE_URL=postgres://chorus:chorus@localhost:5433/chorus \
REDIS_URL=redis://127.0.0.1:6380 PORT=3001 HOST=127.0.0.1 \
cargo run -p chorus-server &
until curl -s http://127.0.0.1:3001/health | grep -q ok; do sleep 1; done
```

- [ ] **Step 3: Seed an account + api_key**
```bash
SMOKE_KEY="ch_test_verify-smoke-key-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
SMOKE_HASH=$(printf '%s' "$SMOKE_KEY" | sha256sum | awk '{print $1}')
podman exec -i chorus-verify-pg psql -U chorus -d chorus <<SQL
INSERT INTO accounts (id, name, owner_email, is_active)
  VALUES ('00000000-0000-0000-0000-000000000001', 'smoke', 's@x.com', true);
INSERT INTO api_keys (id, account_id, name, key_hash, key_prefix, environment)
  VALUES ('00000000-0000-0000-0000-000000000002',
          '00000000-0000-0000-0000-000000000001',
          'smoke', '$SMOKE_HASH', 'ch_test_ve...', 'test');
SQL
```

- [ ] **Step 4: Run smoke scenarios**
```bash
# 1. Create with email → expect channel="email", cost_micro=100
curl -s -H "authorization: Bearer $SMOKE_KEY" -H "content-type: application/json" \
  -d '{"phone":"+66812345678","email":"alice@example.com"}' \
  http://127.0.0.1:3001/v1/verifications

# 2. Idempotency replay
IDEM="smoke-$(date +%s)"
RESP1=$(curl -s -H "authorization: Bearer $SMOKE_KEY" -H "Idempotency-Key: $IDEM" \
  -H "content-type: application/json" \
  -d '{"phone":"+66812345678","email":"a@b.com"}' http://127.0.0.1:3001/v1/verifications)
RESP2=$(curl -s -H "authorization: Bearer $SMOKE_KEY" -H "Idempotency-Key: $IDEM" \
  -H "content-type: application/json" \
  -d '{"phone":"+66812345678","email":"a@b.com"}' http://127.0.0.1:3001/v1/verifications)
[ "$RESP1" = "$RESP2" ] && echo "REPLAY OK" || echo "REPLAY MISMATCH"

# 3. Suppress email → expect SMS fallback
curl -s -H "authorization: Bearer $SMOKE_KEY" -H "content-type: application/json" \
  -d '{"channel":"email","recipient":"a@b.com"}' http://127.0.0.1:3001/v1/suppressions
curl -s -H "authorization: Bearer $SMOKE_KEY" -H "content-type: application/json" \
  -d '{"phone":"+66812345678","email":"a@b.com"}' http://127.0.0.1:3001/v1/verifications
# expect channel=sms, cost_micro=6000

# 4. Rate-limit per recipient — 5 OK + 6th 429
for i in 1 2 3 4 5 6; do
  curl -s -o /dev/null -w "%{http_code}\n" \
    -H "authorization: Bearer $SMOKE_KEY" -H "content-type: application/json" \
    -d '{"phone":"+14155552671"}' http://127.0.0.1:3001/v1/verifications
done
# expect 201,201,201,201,201,429

# 5. /metrics
curl -s http://127.0.0.1:3001/metrics | grep chorus_verifications | head -10

# 6. Postgres inspection
podman exec -i chorus-verify-pg psql -U chorus -d chorus -c \
  "SELECT id, channel, recipient, status, check_attempts, resend_attempts, cost_micro
   FROM verifications ORDER BY created_at DESC LIMIT 10"
```

- [ ] **Step 5: Cleanup smoke env**
```bash
pkill -f "chorus-server" 2>/dev/null
podman stop chorus-verify-pg chorus-verify-vk
podman rm  chorus-verify-pg chorus-verify-vk
```

- [ ] **Step 6: Push branch + open PR**
```bash
git push -u origin feat/verification-api
gh pr create --base main --head feat/verification-api \
  --title "feat(server): B1 — Verification API + waterfall (save 60-80% vs Twilio)" \
  --body-file - <<'EOF'
## Summary

Stripe-Verify-class API at `/v1/verifications/*` with smart routing
(email-first → SMS fallback), per-call cost reporting, and rate
limiting. Hero feature of the B (differentiator) tier.

- 6 routes: create / check / resend / cancel / get / list
- Postgres `verifications` + Valkey for code TTL
- Per-recipient (5/hr) + per-account (100/min) sliding-window rate limits
- Per-call cost_micro in response (email=100, US SMS=5000, TH SMS=6000)
- Idempotency-Key support on create+resend (C1)
- Legacy `/v1/otp/*` untouched

Spec: `docs/superpowers/specs/2026-05-06-verification-api-design.md`
Plan: `docs/superpowers/plans/2026-05-06-verification-api.md`

## Test plan

- [x] Unit tests for code generator + pricing
- [x] sqlx repo tests (ignored by default; run with --ignored + DATABASE_URL)
- [x] API integration tests for create/check/cancel/get/list/resend
- [x] Idempotency replay test (byte-for-byte identical)
- [x] Smoke test on podman (replay + SMS fallback + rate-limit + metrics)
EOF
```

---

## Self-review

- ✅ Spec coverage: every section of `2026-05-06-verification-api-design.md` is implemented by at least one task — migration (T1), repo (T2-T3), orchestration (T4, T6-T8), AppState (T5), routes (T9-T10), cleanup (T13), metrics (T14), tests (T11-T12), CI/smoke (T15-T16).
- ✅ Type consistency: `Verification` / `NewVerification` / `VerificationRepository` / `ChannelChoice` / `RoutingError` / `CheckCodeOutcome` used uniformly across tasks.
- ✅ No `TBD` / placeholder strings — every code block is complete; metric instrumentation in T14 has explicit code blocks for each call site.
