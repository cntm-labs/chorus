# Suppression List MVP Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a per-account, per-channel suppression list populated by chorus-mail bounce notifications and a customer-facing manual API. Reject suppressed sends at API entry with HTTP 422.

**Architecture:** New `suppressions` table keyed on `(account_id, channel, recipient)`. A new `SuppressionRepository` trait with Postgres impl. A hot-path helper `check_suppression()` called from every send route. The existing `/internal/bounces` handler is extended to populate suppressions transactionally with the existing message status update.

**Tech Stack:** Rust, Axum, sqlx (PostgreSQL), async-trait, regex, tokio

**Spec:** `docs/superpowers/specs/2026-04-28-suppression-list-mvp-design.md`

**Testing approach:** This plan uses **in-memory mocks** for repository testing (consistent with the existing `tests/api_test.rs` pattern), not `sqlx::test` fixtures. The spec mentions repository tests with sqlx fixtures; we adapt to the codebase's established mock-based pattern instead. Adding sqlx::test infrastructure would be its own scoped task and is out of scope here.

---

## File Structure

| File | Action | Responsibility |
|---|---|---|
| `services/chorus-server/src/db/migrations/007_create_suppressions.sql` | Create | Postgres schema |
| `services/chorus-server/src/db/mod.rs` | Modify | Add `Suppression`, `NewSuppression`, `SuppressionRepository` trait + module export |
| `services/chorus-server/src/db/suppression.rs` | Create | `PgSuppressionRepository` impl |
| `services/chorus-server/src/suppression.rs` | Create | `normalize()`, `check_suppression()`, `SuppressionRejection` |
| `services/chorus-server/src/lib.rs` | Modify | Re-export `pub mod suppression` |
| `services/chorus-server/src/app.rs` | Modify | Wire `suppression_repo` into `AppState` and `with_repos` |
| `services/chorus-server/src/routes/suppressions.rs` | Create | CRUD handlers (`GET/POST/DELETE /v1/suppressions`) |
| `services/chorus-server/src/routes/mod.rs` | Modify | Add `pub mod suppressions;` |
| `services/chorus-server/src/routes/sms.rs` | Modify | Hot-path `check_suppression` call |
| `services/chorus-server/src/routes/email.rs` | Modify | Hot-path `check_suppression` call |
| `services/chorus-server/src/routes/otp.rs` | Modify | Hot-path `check_suppression` call |
| `services/chorus-server/src/routes/batch.rs` | Modify | Per-entry filter; result entries gain `"suppressed"` status + reason |
| `services/chorus-server/src/routes/internal.rs` | Modify | Extend `handle_bounce` to write suppression + mark message bounced + delivery_event |
| `services/chorus-server/tests/api_test.rs` | Modify | Add `MockSuppressionRepo`, update `with_repos` call, add test cases |

---

### Task 1: Add `suppressions` table migration

**Files:**
- Create: `services/chorus-server/src/db/migrations/007_create_suppressions.sql`

- [ ] **Step 1: Write the migration**

```sql
-- services/chorus-server/src/db/migrations/007_create_suppressions.sql
CREATE TABLE suppressions (
    account_id  UUID NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    channel     TEXT NOT NULL CHECK (channel IN ('sms', 'email')),
    recipient   TEXT NOT NULL,
    reason      TEXT NOT NULL CHECK (reason IN ('hard_bounce', 'manual')),
    source      TEXT NOT NULL CHECK (source IN ('chorus-mail', 'api')),
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (account_id, channel, recipient)
);
```

- [ ] **Step 2: Run `cargo check -p chorus-server`**

Expected: compiles cleanly. (Migrations are embedded via `sqlx::migrate!()` so the file just needs to be in the directory.)

- [ ] **Step 3: Commit**

```bash
git add services/chorus-server/src/db/migrations/007_create_suppressions.sql
git commit -m "feat(server): add suppressions table migration"
```

---

### Task 2: Add suppression types and trait to `db/mod.rs`

**Files:**
- Modify: `services/chorus-server/src/db/mod.rs`

- [ ] **Step 1: Add module export**

In `services/chorus-server/src/db/mod.rs`, add a new line after `pub mod webhook;` (line 5):

```rust
pub mod suppression;
```

- [ ] **Step 2: Add `Suppression`, `NewSuppression`, and `SuppressionRepository` trait at end of file**

Append to `services/chorus-server/src/db/mod.rs`:

```rust
/// A suppression list entry for a recipient that should not receive messages.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Suppression {
    pub account_id: Uuid,
    pub channel: String,
    pub recipient: String,
    pub reason: String,
    pub source: String,
    pub created_at: DateTime<Utc>,
}

/// Parameters for inserting a new suppression entry.
pub struct NewSuppression {
    pub account_id: Uuid,
    pub channel: String,
    pub recipient: String,
    pub reason: String,
    pub source: String,
}

/// Suppression list management.
#[async_trait]
pub trait SuppressionRepository: Send + Sync {
    /// Returns the suppression `reason` if `recipient` is suppressed for the given account+channel.
    async fn is_suppressed(
        &self,
        account_id: Uuid,
        channel: &str,
        recipient: &str,
    ) -> Result<Option<String>, DbError>;

    /// Insert a suppression. Idempotent: existing rows are left untouched.
    async fn add(&self, entry: &NewSuppression) -> Result<(), DbError>;

    /// Remove a suppression. Returns `true` if a row was deleted.
    async fn remove(
        &self,
        account_id: Uuid,
        channel: &str,
        recipient: &str,
    ) -> Result<bool, DbError>;

    /// List suppressions for an account, optionally filtered by channel, with pagination.
    async fn list(
        &self,
        account_id: Uuid,
        channel: Option<&str>,
        pagination: &Pagination,
    ) -> Result<Vec<Suppression>, DbError>;
}
```

- [ ] **Step 3: Run `cargo check -p chorus-server`**

Expected: fails because `db::suppression` module doesn't exist yet. Continue to Task 3 to satisfy.

---

### Task 3: Implement `PgSuppressionRepository`

**Files:**
- Create: `services/chorus-server/src/db/suppression.rs`

- [ ] **Step 1: Create the file**

```rust
// services/chorus-server/src/db/suppression.rs
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
```

- [ ] **Step 2: Run `cargo check -p chorus-server`**

Expected: compiles cleanly.

- [ ] **Step 3: Commit**

```bash
git add services/chorus-server/src/db/mod.rs services/chorus-server/src/db/suppression.rs
git commit -m "feat(server): add SuppressionRepository trait and Postgres impl"
```

---

### Task 4: Wire `suppression_repo` into `AppState`

**Files:**
- Modify: `services/chorus-server/src/app.rs`

- [ ] **Step 1: Add imports**

In `services/chorus-server/src/app.rs`, replace the existing import block at lines 9-16 with:

```rust
use crate::config::Config;
use crate::db::billing::{BillingRepository, PgBillingRepository};
use crate::db::postgres::PgRepository;
use crate::db::provider_config::PgProviderConfigRepository;
use crate::db::suppression::PgSuppressionRepository;
use crate::db::webhook::PgWebhookRepository;
use crate::db::{
    AccountRepository, AdminKeyRepository, AdminRepository, ApiKeyRepository, MessageRepository,
    PgAdminRepository, ProviderConfigRepository, SuppressionRepository, WebhookRepository,
};
use crate::routes;
```

- [ ] **Step 2: Add `suppression_repo` field**

In the `AppState` struct (after `webhook_repo`, before `billing_repo`), add:

```rust
    /// Suppression list repository.
    suppression_repo: Arc<dyn SuppressionRepository>,
```

- [ ] **Step 3: Construct `suppression_repo` in `AppState::new`**

In `pub fn new(...)` (around line 49), after `let webhook_repo = ...`, add:

```rust
        let suppression_repo = Arc::new(PgSuppressionRepository::new(db.clone()));
```

And add `suppression_repo,` to the struct literal (alphabetical order alongside the other repos).

- [ ] **Step 4: Update `with_repos` test constructor signature**

Replace `pub fn with_repos(...)` (currently lines 72-95) with:

```rust
    /// Create app state with custom repositories (for testing).
    pub fn with_repos(
        redis: redis::Client,
        config: Arc<Config>,
        account_repo: Arc<dyn AccountRepository>,
        message_repo: Arc<dyn MessageRepository>,
        api_key_repo: Arc<dyn ApiKeyRepository>,
        provider_config_repo: Arc<dyn ProviderConfigRepository>,
        webhook_repo: Arc<dyn WebhookRepository>,
        suppression_repo: Arc<dyn SuppressionRepository>,
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
            billing_repo: Arc::new(crate::db::billing::NullBillingRepository),
            admin_key_repo: Arc::new(NullAdminKeyRepository),
            admin_repo: Arc::new(NullAdminRepository),
        }
    }
```

- [ ] **Step 5: Add accessor**

After `pub fn webhook_repo(...)` (around line 118), add:

```rust
    /// Access the suppression repository.
    pub fn suppression_repo(&self) -> Arc<dyn SuppressionRepository> {
        Arc::clone(&self.suppression_repo)
    }
```

- [ ] **Step 6: Run `cargo check -p chorus-server`**

Expected: fails in `tests/api_test.rs` because the `with_repos` call there doesn't pass a suppression_repo. We'll fix that in Task 5.

---

### Task 5: Add `MockSuppressionRepo` and update test bootstrap

**Files:**
- Modify: `services/chorus-server/tests/api_test.rs`

- [ ] **Step 1: Add `SuppressionRepository` and types to imports**

In `services/chorus-server/tests/api_test.rs`, replace the import block (around lines 13-18) with:

```rust
use chorus_server::db::{
    Account, AccountRepository, ApiKey, ApiKeyRepository, DbError, DeliveryEvent, Message,
    MessageRepository, NewMessage, NewProviderConfig, NewSuppression, NewWebhook, Pagination,
    ProviderConfig, ProviderConfigRepository, Suppression, SuppressionRepository, Webhook,
    WebhookRepository,
};
```

- [ ] **Step 2: Add `MockSuppressionRepo` after `MockWebhookRepo` (around line 197)**

```rust
struct MockSuppressionRepo {
    entries: Mutex<Vec<Suppression>>,
}

impl MockSuppressionRepo {
    fn new() -> Self {
        Self {
            entries: Mutex::new(Vec::new()),
        }
    }
}

#[async_trait]
impl SuppressionRepository for MockSuppressionRepo {
    async fn is_suppressed(
        &self,
        account_id: Uuid,
        channel: &str,
        recipient: &str,
    ) -> Result<Option<String>, DbError> {
        let entries = self.entries.lock().unwrap();
        Ok(entries
            .iter()
            .find(|e| e.account_id == account_id && e.channel == channel && e.recipient == recipient)
            .map(|e| e.reason.clone()))
    }

    async fn add(&self, entry: &NewSuppression) -> Result<(), DbError> {
        let mut entries = self.entries.lock().unwrap();
        let exists = entries.iter().any(|e| {
            e.account_id == entry.account_id
                && e.channel == entry.channel
                && e.recipient == entry.recipient
        });
        if !exists {
            entries.push(Suppression {
                account_id: entry.account_id,
                channel: entry.channel.clone(),
                recipient: entry.recipient.clone(),
                reason: entry.reason.clone(),
                source: entry.source.clone(),
                created_at: Utc::now(),
            });
        }
        Ok(())
    }

    async fn remove(
        &self,
        account_id: Uuid,
        channel: &str,
        recipient: &str,
    ) -> Result<bool, DbError> {
        let mut entries = self.entries.lock().unwrap();
        let before = entries.len();
        entries.retain(|e| {
            !(e.account_id == account_id && e.channel == channel && e.recipient == recipient)
        });
        Ok(entries.len() < before)
    }

    async fn list(
        &self,
        account_id: Uuid,
        channel: Option<&str>,
        pagination: &Pagination,
    ) -> Result<Vec<Suppression>, DbError> {
        let entries = self.entries.lock().unwrap();
        let filtered: Vec<_> = entries
            .iter()
            .filter(|e| e.account_id == account_id)
            .filter(|e| channel.map_or(true, |c| e.channel == c))
            .skip(pagination.offset as usize)
            .take(pagination.limit as usize)
            .cloned()
            .collect();
        Ok(filtered)
    }
}
```

- [ ] **Step 3: Update `test_state()` to construct and pass `suppression_repo`**

Replace the body of `fn test_state() -> Arc<AppState>` so it ends with:

```rust
    let message_repo = Arc::new(MockMessageRepo::new());
    let api_key_repo = Arc::new(MockApiKeyRepo);
    let provider_config_repo = Arc::new(MockProviderConfigRepo);
    let webhook_repo = Arc::new(MockWebhookRepo);
    let suppression_repo = Arc::new(MockSuppressionRepo::new());

    let redis = redis::Client::open("redis://127.0.0.1:6379").unwrap();
    let config = Arc::new(Config::from_env());
    Arc::new(AppState::with_repos(
        redis,
        config,
        account_repo,
        message_repo,
        api_key_repo,
        provider_config_repo,
        webhook_repo,
        suppression_repo,
    ))
}
```

- [ ] **Step 4: Run `cargo test -p chorus-server`**

Expected: all existing tests still pass.

- [ ] **Step 5: Commit**

```bash
git add services/chorus-server/src/app.rs services/chorus-server/tests/api_test.rs
git commit -m "feat(server): wire SuppressionRepository into AppState"
```

---

### Task 6: Write `normalize()` with unit tests (TDD)

**Files:**
- Create: `services/chorus-server/src/suppression.rs`
- Modify: `services/chorus-server/src/lib.rs`

- [ ] **Step 1: Add unit-test stub file with failing tests**

Create `services/chorus-server/src/suppression.rs`:

```rust
//! Suppression list helpers: recipient normalization and hot-path lookup.

use std::sync::Arc;
use uuid::Uuid;

use crate::app::AppState;
use crate::db::DbError;

/// Why a normalize call failed.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum NormalizeError {
    #[error("invalid E.164 phone number")]
    InvalidE164,
    #[error("unknown channel: {0}")]
    UnknownChannel(String),
}

/// Reasons a hot-path send may be rejected by the suppression layer.
#[derive(Debug, thiserror::Error)]
pub enum SuppressionRejection {
    #[error("recipient is suppressed: {reason}")]
    Suppressed { reason: String },
    #[error("invalid recipient")]
    InvalidRecipient,
    #[error("database error: {0}")]
    Db(#[from] DbError),
}

/// Normalize a recipient to its canonical form for storage and lookup.
pub fn normalize(channel: &str, recipient: &str) -> Result<String, NormalizeError> {
    match channel {
        "email" => Ok(recipient.trim().to_lowercase()),
        "sms" => {
            let r = recipient.trim();
            // E.164: leading '+', country code 1-9, total 8-15 digits.
            let re = regex::Regex::new(r"^\+[1-9]\d{1,14}$").expect("valid regex");
            if re.is_match(r) {
                Ok(r.to_string())
            } else {
                Err(NormalizeError::InvalidE164)
            }
        }
        other => Err(NormalizeError::UnknownChannel(other.to_string())),
    }
}

/// Hot-path check: returns Ok(()) if the recipient is allowed to receive a message.
pub async fn check_suppression(
    state: &Arc<AppState>,
    account_id: Uuid,
    channel: &str,
    recipient: &str,
) -> Result<(), SuppressionRejection> {
    let normalized = normalize(channel, recipient)
        .map_err(|_| SuppressionRejection::InvalidRecipient)?;
    match state
        .suppression_repo()
        .is_suppressed(account_id, channel, &normalized)
        .await?
    {
        Some(reason) => Err(SuppressionRejection::Suppressed { reason }),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn email_lowercases_and_trims() {
        assert_eq!(
            normalize("email", "  Alice@Example.COM  ").unwrap(),
            "alice@example.com"
        );
    }

    #[test]
    fn sms_passes_valid_e164() {
        assert_eq!(normalize("sms", "+66812345678").unwrap(), "+66812345678");
    }

    #[test]
    fn sms_trims_then_validates() {
        assert_eq!(normalize("sms", "  +14155552671 ").unwrap(), "+14155552671");
    }

    #[test]
    fn sms_rejects_no_plus() {
        assert_eq!(
            normalize("sms", "14155552671").unwrap_err(),
            NormalizeError::InvalidE164
        );
    }

    #[test]
    fn sms_rejects_leading_zero_country_code() {
        assert_eq!(
            normalize("sms", "+0123456789").unwrap_err(),
            NormalizeError::InvalidE164
        );
    }

    #[test]
    fn sms_rejects_letters() {
        assert_eq!(
            normalize("sms", "+1abc4155552671").unwrap_err(),
            NormalizeError::InvalidE164
        );
    }

    #[test]
    fn sms_rejects_too_short() {
        assert_eq!(
            normalize("sms", "+1").unwrap_err(),
            NormalizeError::InvalidE164
        );
    }

    #[test]
    fn unknown_channel_errors() {
        match normalize("voice", "anything") {
            Err(NormalizeError::UnknownChannel(c)) => assert_eq!(c, "voice"),
            other => panic!("unexpected: {other:?}"),
        }
    }
}
```

- [ ] **Step 2: Re-export from lib.rs**

In `services/chorus-server/src/lib.rs`, add:

```rust
pub mod suppression;
```

(Place alongside the other `pub mod` lines.)

- [ ] **Step 3: Verify `regex` is in Cargo.toml**

Run `grep '^regex' services/chorus-server/Cargo.toml`. If not present, add it under `[dependencies]`:

```toml
regex = "1"
```

- [ ] **Step 4: Run unit tests**

Run: `cargo test -p chorus-server --lib suppression::tests`

Expected: 8 tests pass.

- [ ] **Step 5: Commit**

```bash
git add services/chorus-server/src/suppression.rs services/chorus-server/src/lib.rs services/chorus-server/Cargo.toml
git commit -m "feat(server): add suppression normalize and hot-path helpers"
```

---

### Task 7: Add `routes/suppressions.rs` (customer-facing CRUD)

**Files:**
- Create: `services/chorus-server/src/routes/suppressions.rs`
- Modify: `services/chorus-server/src/routes/mod.rs`
- Modify: `services/chorus-server/src/app.rs` (router wiring)

- [ ] **Step 1: Add module export**

In `services/chorus-server/src/routes/mod.rs`, add:

```rust
pub mod suppressions;
```

(Alongside the other `pub mod` lines.)

- [ ] **Step 2: Create the route file**

```rust
// services/chorus-server/src/routes/suppressions.rs
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::app::AppState;
use crate::auth::api_key::AccountContext;
use crate::db::{NewSuppression, Pagination, Suppression};
use crate::suppression::normalize;

const DEFAULT_LIMIT: i64 = 20;
const MAX_LIMIT: i64 = 100;

/// Query params for `GET /v1/suppressions`.
#[derive(Deserialize)]
pub struct ListParams {
    pub channel: Option<String>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

/// Body for `POST /v1/suppressions`.
#[derive(Deserialize)]
pub struct CreateSuppressionRequest {
    pub channel: String,
    pub recipient: String,
}

/// Wire-form representation of a suppression.
#[derive(Serialize)]
pub struct SuppressionResponse {
    pub channel: String,
    pub recipient: String,
    pub reason: String,
    pub source: String,
    pub created_at: String,
}

/// Paginated list response.
#[derive(Serialize)]
pub struct SuppressionListResponse {
    pub data: Vec<SuppressionResponse>,
    pub limit: i64,
    pub offset: i64,
}

impl From<Suppression> for SuppressionResponse {
    fn from(s: Suppression) -> Self {
        Self {
            channel: s.channel,
            recipient: s.recipient,
            reason: s.reason,
            source: s.source,
            created_at: s.created_at.to_rfc3339(),
        }
    }
}

/// `GET /v1/suppressions`
pub async fn list_suppressions(
    State(state): State<Arc<AppState>>,
    ctx: AccountContext,
    Query(params): Query<ListParams>,
) -> Result<Json<SuppressionListResponse>, (StatusCode, String)> {
    let limit = params.limit.unwrap_or(DEFAULT_LIMIT).min(MAX_LIMIT);
    let offset = params.offset.unwrap_or(0);
    let pagination = Pagination { limit, offset };

    let entries = state
        .suppression_repo()
        .list(ctx.account_id, params.channel.as_deref(), &pagination)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(SuppressionListResponse {
        data: entries.into_iter().map(SuppressionResponse::from).collect(),
        limit,
        offset,
    }))
}

/// `POST /v1/suppressions`
pub async fn create_suppression(
    State(state): State<Arc<AppState>>,
    ctx: AccountContext,
    Json(req): Json<CreateSuppressionRequest>,
) -> Result<(StatusCode, Json<SuppressionResponse>), (StatusCode, String)> {
    let normalized = normalize(&req.channel, &req.recipient)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;

    let entry = NewSuppression {
        account_id: ctx.account_id,
        channel: req.channel.clone(),
        recipient: normalized.clone(),
        reason: "manual".into(),
        source: "api".into(),
    };

    state
        .suppression_repo()
        .add(&entry)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let resp = SuppressionResponse {
        channel: req.channel,
        recipient: normalized,
        reason: "manual".into(),
        source: "api".into(),
        created_at: chrono::Utc::now().to_rfc3339(),
    };

    Ok((StatusCode::CREATED, Json(resp)))
}

/// `DELETE /v1/suppressions/{channel}/{recipient}`
pub async fn delete_suppression(
    State(state): State<Arc<AppState>>,
    ctx: AccountContext,
    Path((channel, recipient)): Path<(String, String)>,
) -> Result<StatusCode, (StatusCode, String)> {
    let normalized = normalize(&channel, &recipient)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;

    let removed = state
        .suppression_repo()
        .remove(ctx.account_id, &channel, &normalized)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    if removed {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err((StatusCode::NOT_FOUND, "suppression not found".into()))
    }
}
```

- [ ] **Step 3: Register routes in `app.rs`**

In `services/chorus-server/src/app.rs::create_router_with_metrics`, add three new `.route(...)` calls. Place them adjacent to the `/v1/webhooks` block (around line 184). Insert before the `.route("/v1/sms/send-batch", ...)` line:

```rust
        .route(
            "/v1/suppressions",
            get(routes::suppressions::list_suppressions)
                .post(routes::suppressions::create_suppression),
        )
        .route(
            "/v1/suppressions/{channel}/{recipient}",
            delete(routes::suppressions::delete_suppression),
        )
```

- [ ] **Step 4: Run `cargo check -p chorus-server`**

Expected: compiles cleanly.

- [ ] **Step 5: Commit**

```bash
git add services/chorus-server/src/routes/suppressions.rs services/chorus-server/src/routes/mod.rs services/chorus-server/src/app.rs
git commit -m "feat(server): add /v1/suppressions CRUD endpoints"
```

---

### Task 8: Hot-path integration tests for `/v1/suppressions` (TDD for Task 9–10)

**Files:**
- Modify: `services/chorus-server/tests/api_test.rs`

- [ ] **Step 1: Write a helper that signs requests with the test API key**

If not already present, look at the existing test for an authed request pattern. The existing tests use the literal `TEST_API_KEY` constant directly in the `Authorization` header. The new tests will follow the same pattern.

- [ ] **Step 2: Append integration tests at the end of `tests/api_test.rs`**

```rust
#[tokio::test]
async fn list_suppressions_empty_returns_200() {
    let app = create_router(test_state());

    let resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/suppressions")
                .header("authorization", format!("Bearer {TEST_API_KEY}"))
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let body = response_body(resp).await;
    assert_eq!(body["data"], serde_json::json!([]));
    assert_eq!(body["limit"], 20);
    assert_eq!(body["offset"], 0);
}

#[tokio::test]
async fn create_suppression_normalizes_email_and_returns_201() {
    let app = create_router(test_state());

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/suppressions")
                .header("authorization", format!("Bearer {TEST_API_KEY}"))
                .header("content-type", "application/json")
                .body(axum::body::Body::from(
                    r#"{"channel":"email","recipient":"  Alice@Example.COM "}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::CREATED);
    let body = response_body(resp).await;
    assert_eq!(body["recipient"], "alice@example.com");
    assert_eq!(body["reason"], "manual");
    assert_eq!(body["source"], "api");
}

#[tokio::test]
async fn create_suppression_rejects_bad_e164() {
    let app = create_router(test_state());

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/suppressions")
                .header("authorization", format!("Bearer {TEST_API_KEY}"))
                .header("content-type", "application/json")
                .body(axum::body::Body::from(
                    r#"{"channel":"sms","recipient":"0812345678"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn delete_suppression_round_trip() {
    let state = test_state();
    let app = create_router(Arc::clone(&state));

    // Add
    let add = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/suppressions")
                .header("authorization", format!("Bearer {TEST_API_KEY}"))
                .header("content-type", "application/json")
                .body(axum::body::Body::from(
                    r#"{"channel":"email","recipient":"bob@example.com"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(add.status(), StatusCode::CREATED);

    // Delete
    let del = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/v1/suppressions/email/bob@example.com")
                .header("authorization", format!("Bearer {TEST_API_KEY}"))
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(del.status(), StatusCode::NO_CONTENT);

    // Delete again → 404
    let del2 = app
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/v1/suppressions/email/bob@example.com")
                .header("authorization", format!("Bearer {TEST_API_KEY}"))
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(del2.status(), StatusCode::NOT_FOUND);
}
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p chorus-server --test api_test list_suppressions_empty_returns_200 create_suppression_normalizes_email_and_returns_201 create_suppression_rejects_bad_e164 delete_suppression_round_trip`

Expected: all 4 tests pass.

- [ ] **Step 4: Commit**

```bash
git add services/chorus-server/tests/api_test.rs
git commit -m "test(server): add /v1/suppressions CRUD integration tests"
```

---

### Task 9: Wire `check_suppression` into `/v1/sms/send`

**Files:**
- Modify: `services/chorus-server/src/routes/sms.rs`
- Modify: `services/chorus-server/tests/api_test.rs`

- [ ] **Step 1: Write the failing test**

Append to `tests/api_test.rs`:

```rust
#[tokio::test]
async fn sms_send_to_suppressed_recipient_returns_422() {
    let state = test_state();
    let app = create_router(Arc::clone(&state));

    // Pre-populate suppression
    let _ = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/suppressions")
                .header("authorization", format!("Bearer {TEST_API_KEY}"))
                .header("content-type", "application/json")
                .body(axum::body::Body::from(
                    r#"{"channel":"sms","recipient":"+14155552671"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/sms/send")
                .header("authorization", format!("Bearer {TEST_API_KEY}"))
                .header("content-type", "application/json")
                .body(axum::body::Body::from(
                    r#"{"to":"+14155552671","body":"hi"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let body = response_body(resp).await;
    assert_eq!(body["error"]["code"], "recipient_suppressed");
    assert_eq!(body["error"]["reason"], "manual");
}
```

- [ ] **Step 2: Run the test to confirm it fails**

Run: `cargo test -p chorus-server --test api_test sms_send_to_suppressed_recipient_returns_422`

Expected: FAIL — the route doesn't check suppression yet (likely returns 202 or 500).

- [ ] **Step 3: Add suppression check to the SMS handler**

Replace the body of `pub async fn send_sms(...)` in `services/chorus-server/src/routes/sms.rs` (lines 32-73) with:

```rust
pub async fn send_sms(
    State(state): State<Arc<AppState>>,
    ctx: AccountContext,
    Json(req): Json<SendSmsRequest>,
) -> Result<(StatusCode, Json<SendResponse>), (StatusCode, axum::Json<serde_json::Value>)> {
    if let Err(e) = crate::suppression::check_suppression(&state, ctx.account_id, "sms", &req.to).await {
        return Err(suppression_error_response(e));
    }

    let new_msg = NewMessage {
        account_id: ctx.account_id,
        api_key_id: ctx.key_id,
        channel: "sms".into(),
        sender: req.from,
        recipient: req.to,
        subject: None,
        body: req.body,
        environment: ctx.environment,
    };

    let message = state
        .message_repo()
        .insert(&new_msg)
        .await
        .map_err(|e| internal_error(e.to_string()))?;

    let job = SendJob {
        message_id: message.id,
        account_id: message.account_id,
        channel: "sms".into(),
        environment: message.environment.clone(),
        attempt: 0,
    };
    crate::queue::enqueue::notify(&state, &job)
        .await
        .map_err(|e| internal_error(e.to_string()))?;

    Ok((
        StatusCode::ACCEPTED,
        Json(SendResponse {
            message_id: message.id,
            status: "queued",
        }),
    ))
}

fn suppression_error_response(
    err: crate::suppression::SuppressionRejection,
) -> (StatusCode, axum::Json<serde_json::Value>) {
    use crate::suppression::SuppressionRejection;
    match err {
        SuppressionRejection::Suppressed { reason } => (
            StatusCode::UNPROCESSABLE_ENTITY,
            axum::Json(serde_json::json!({
                "error": {
                    "code": "recipient_suppressed",
                    "message": "Recipient is on the suppression list",
                    "reason": reason,
                }
            })),
        ),
        SuppressionRejection::InvalidRecipient => (
            StatusCode::BAD_REQUEST,
            axum::Json(serde_json::json!({
                "error": { "code": "invalid_recipient" }
            })),
        ),
        SuppressionRejection::Db(e) => internal_error(e.to_string()),
    }
}

fn internal_error(msg: String) -> (StatusCode, axum::Json<serde_json::Value>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        axum::Json(serde_json::json!({ "error": { "message": msg } })),
    )
}
```

Note: the return type changes from `(StatusCode, String)` to `(StatusCode, axum::Json<serde_json::Value>)` so we can return structured error bodies. This matches the spec's wire format.

- [ ] **Step 4: Run the test to confirm it passes**

Run: `cargo test -p chorus-server --test api_test sms_send_to_suppressed_recipient_returns_422`

Expected: PASS.

- [ ] **Step 5: Run all tests to ensure no regression**

Run: `cargo test -p chorus-server --test api_test`

Expected: all pass. (The change to error return type may break other tests that asserted on the SMS error body — adjust them to match the new shape if so.)

- [ ] **Step 6: Commit**

```bash
git add services/chorus-server/src/routes/sms.rs services/chorus-server/tests/api_test.rs
git commit -m "feat(server): suppress check at /v1/sms/send"
```

---

### Task 10: Wire `check_suppression` into `/v1/email/send`

**Files:**
- Modify: `services/chorus-server/src/routes/email.rs`
- Modify: `services/chorus-server/tests/api_test.rs`

- [ ] **Step 1: Write the failing test**

Append to `tests/api_test.rs`:

```rust
#[tokio::test]
async fn email_send_to_suppressed_recipient_returns_422() {
    let app = create_router(test_state());

    let _ = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/suppressions")
                .header("authorization", format!("Bearer {TEST_API_KEY}"))
                .header("content-type", "application/json")
                .body(axum::body::Body::from(
                    r#"{"channel":"email","recipient":"alice@example.com"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/email/send")
                .header("authorization", format!("Bearer {TEST_API_KEY}"))
                .header("content-type", "application/json")
                .body(axum::body::Body::from(
                    r#"{"to":"ALICE@example.com","subject":"hi","body":"hi"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let body = response_body(resp).await;
    assert_eq!(body["error"]["code"], "recipient_suppressed");
}
```

This test also verifies that the case-insensitive lookup works (suppression is `alice@example.com`, send is `ALICE@example.com`).

- [ ] **Step 2: Run the test to confirm it fails**

Run: `cargo test -p chorus-server --test api_test email_send_to_suppressed_recipient_returns_422`

Expected: FAIL.

- [ ] **Step 3: Add suppression check to the email handler**

Replace `services/chorus-server/src/routes/email.rs` with:

```rust
use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use serde::Deserialize;
use std::sync::Arc;

use crate::app::AppState;
use crate::auth::api_key::AccountContext;
use crate::db::NewMessage;
use crate::queue::SendJob;
use crate::routes::sms::SendResponse;

/// Email send request body.
#[derive(Deserialize)]
pub struct SendEmailRequest {
    /// Recipient email address.
    pub to: String,
    /// Email subject line.
    pub subject: String,
    /// Email body (HTML or plain text).
    pub body: String,
    /// Optional sender address.
    pub from: Option<String>,
}

/// Queue an email message for delivery. Returns 202 Accepted.
pub async fn send_email(
    State(state): State<Arc<AppState>>,
    ctx: AccountContext,
    Json(req): Json<SendEmailRequest>,
) -> Result<(StatusCode, Json<SendResponse>), (StatusCode, axum::Json<serde_json::Value>)> {
    if let Err(e) =
        crate::suppression::check_suppression(&state, ctx.account_id, "email", &req.to).await
    {
        return Err(map_suppression_error(e));
    }

    let new_msg = NewMessage {
        account_id: ctx.account_id,
        api_key_id: ctx.key_id,
        channel: "email".into(),
        sender: req.from,
        recipient: req.to,
        subject: Some(req.subject),
        body: req.body,
        environment: ctx.environment,
    };

    let message = state
        .message_repo()
        .insert(&new_msg)
        .await
        .map_err(|e| internal_error(e.to_string()))?;

    let job = SendJob {
        message_id: message.id,
        account_id: message.account_id,
        channel: "email".into(),
        environment: message.environment.clone(),
        attempt: 0,
    };
    crate::queue::enqueue::notify(&state, &job)
        .await
        .map_err(|e| internal_error(e.to_string()))?;

    Ok((
        StatusCode::ACCEPTED,
        Json(SendResponse {
            message_id: message.id,
            status: "queued",
        }),
    ))
}

fn map_suppression_error(
    err: crate::suppression::SuppressionRejection,
) -> (StatusCode, axum::Json<serde_json::Value>) {
    use crate::suppression::SuppressionRejection;
    match err {
        SuppressionRejection::Suppressed { reason } => (
            StatusCode::UNPROCESSABLE_ENTITY,
            axum::Json(serde_json::json!({
                "error": {
                    "code": "recipient_suppressed",
                    "message": "Recipient is on the suppression list",
                    "reason": reason,
                }
            })),
        ),
        SuppressionRejection::InvalidRecipient => (
            StatusCode::BAD_REQUEST,
            axum::Json(serde_json::json!({
                "error": { "code": "invalid_recipient" }
            })),
        ),
        SuppressionRejection::Db(e) => internal_error(e.to_string()),
    }
}

fn internal_error(msg: String) -> (StatusCode, axum::Json<serde_json::Value>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        axum::Json(serde_json::json!({ "error": { "message": msg } })),
    )
}
```

- [ ] **Step 4: Run the test**

Run: `cargo test -p chorus-server --test api_test email_send_to_suppressed_recipient_returns_422`

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add services/chorus-server/src/routes/email.rs services/chorus-server/tests/api_test.rs
git commit -m "feat(server): suppress check at /v1/email/send"
```

---

### Task 11: Refactor — extract `map_suppression_error` to `suppression.rs` (DRY)

**Files:**
- Modify: `services/chorus-server/src/suppression.rs`
- Modify: `services/chorus-server/src/routes/sms.rs`
- Modify: `services/chorus-server/src/routes/email.rs`

- [ ] **Step 1: Move helper into `suppression.rs`**

Append to `services/chorus-server/src/suppression.rs`:

```rust
use axum::http::StatusCode;

/// Convert a `SuppressionRejection` into an HTTP error response body.
pub fn rejection_response(
    err: SuppressionRejection,
) -> (StatusCode, axum::Json<serde_json::Value>) {
    match err {
        SuppressionRejection::Suppressed { reason } => (
            StatusCode::UNPROCESSABLE_ENTITY,
            axum::Json(serde_json::json!({
                "error": {
                    "code": "recipient_suppressed",
                    "message": "Recipient is on the suppression list",
                    "reason": reason,
                }
            })),
        ),
        SuppressionRejection::InvalidRecipient => (
            StatusCode::BAD_REQUEST,
            axum::Json(serde_json::json!({
                "error": { "code": "invalid_recipient" }
            })),
        ),
        SuppressionRejection::Db(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            axum::Json(serde_json::json!({ "error": { "message": e.to_string() } })),
        ),
    }
}
```

- [ ] **Step 2: Replace local helpers in sms.rs and email.rs**

In `routes/sms.rs`, remove the local `suppression_error_response` and `internal_error` helpers. Update the call site to:

```rust
    if let Err(e) = crate::suppression::check_suppression(&state, ctx.account_id, "sms", &req.to).await {
        return Err(crate::suppression::rejection_response(e));
    }
```

For the `internal_error` calls inside the same handler, replace with an inline closure:

```rust
    let message = state
        .message_repo()
        .insert(&new_msg)
        .await
        .map_err(|e| (
            StatusCode::INTERNAL_SERVER_ERROR,
            axum::Json(serde_json::json!({ "error": { "message": e.to_string() } })),
        ))?;
```

Apply the same simplification to `routes/email.rs`.

- [ ] **Step 3: Run all tests**

Run: `cargo test -p chorus-server`

Expected: all existing tests still pass.

- [ ] **Step 4: Commit**

```bash
git add services/chorus-server/src/suppression.rs services/chorus-server/src/routes/sms.rs services/chorus-server/src/routes/email.rs
git commit -m "refactor(server): centralize suppression error response"
```

---

### Task 12: Wire `check_suppression` into `/v1/otp/send`

**Files:**
- Modify: `services/chorus-server/src/routes/otp.rs`
- Modify: `services/chorus-server/tests/api_test.rs`

- [ ] **Step 1: Write the failing test**

Append to `tests/api_test.rs`:

```rust
#[tokio::test]
async fn otp_send_to_suppressed_email_returns_422() {
    let app = create_router(test_state());

    let _ = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/suppressions")
                .header("authorization", format!("Bearer {TEST_API_KEY}"))
                .header("content-type", "application/json")
                .body(axum::body::Body::from(
                    r#"{"channel":"email","recipient":"otp@example.com"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/otp/send")
                .header("authorization", format!("Bearer {TEST_API_KEY}"))
                .header("content-type", "application/json")
                .body(axum::body::Body::from(
                    r#"{"to":"otp@example.com"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);
}
```

- [ ] **Step 2: Run to confirm it fails**

Run: `cargo test -p chorus-server --test api_test otp_send_to_suppressed_email_returns_422`

Expected: FAIL.

- [ ] **Step 3: Add suppression check to OTP handler**

In `services/chorus-server/src/routes/otp.rs`, modify `pub async fn send_otp(...)`:

Change the return type to `Result<(StatusCode, Json<SendOtpResponse>), (StatusCode, axum::Json<serde_json::Value>)>`.

Insert after the existing `let code = ...` line, but before storing in Redis:

```rust
    let channel = if req.to.contains('@') { "email" } else { "sms" };
    if let Err(e) =
        crate::suppression::check_suppression(&state, _ctx.account_id, channel, &req.to).await
    {
        return Err(crate::suppression::rejection_response(e));
    }
```

Update the channel assignment in the `NewMessage` block to reuse the variable instead of re-evaluating:

```rust
        channel: channel.into(),
```

Replace all subsequent error-mapping calls (`(StatusCode::INTERNAL_SERVER_ERROR, e.to_string())`) with the JSON-shaped form:

```rust
.map_err(|e| (
    StatusCode::INTERNAL_SERVER_ERROR,
    axum::Json(serde_json::json!({ "error": { "message": e.to_string() } })),
))?
```

- [ ] **Step 4: Run the test**

Run: `cargo test -p chorus-server --test api_test otp_send_to_suppressed_email_returns_422`

Expected: PASS.

- [ ] **Step 5: Run all tests**

Run: `cargo test -p chorus-server`

Expected: all pass.

- [ ] **Step 6: Commit**

```bash
git add services/chorus-server/src/routes/otp.rs services/chorus-server/tests/api_test.rs
git commit -m "feat(server): suppress check at /v1/otp/send"
```

---

### Task 13: Wire per-entry filter into batch routes

**Files:**
- Modify: `services/chorus-server/src/routes/batch.rs`
- Modify: `services/chorus-server/tests/api_test.rs`

- [ ] **Step 1: Write the failing test**

Append to `tests/api_test.rs`:

```rust
#[tokio::test]
async fn email_batch_with_suppressed_recipient_returns_207() {
    let app = create_router(test_state());

    // Suppress one recipient
    let _ = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/suppressions")
                .header("authorization", format!("Bearer {TEST_API_KEY}"))
                .header("content-type", "application/json")
                .body(axum::body::Body::from(
                    r#"{"channel":"email","recipient":"bad@example.com"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/email/send-batch")
                .header("authorization", format!("Bearer {TEST_API_KEY}"))
                .header("content-type", "application/json")
                .body(axum::body::Body::from(
                    r#"{"recipients":[
                        {"to":"good@example.com","subject":"x","body":"y"},
                        {"to":"bad@example.com","subject":"x","body":"y"}
                    ]}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::MULTI_STATUS);
    let body = response_body(resp).await;
    let messages = body["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 2);
    let suppressed: Vec<_> = messages.iter().filter(|m| m["status"] == "suppressed").collect();
    assert_eq!(suppressed.len(), 1);
    assert_eq!(suppressed[0]["to"], "bad@example.com");
    assert_eq!(suppressed[0]["reason"], "manual");
}
```

- [ ] **Step 2: Run the test to confirm it fails**

Run: `cargo test -p chorus-server --test api_test email_batch_with_suppressed_recipient_returns_207`

Expected: FAIL — current batch enqueues all and returns 202.

- [ ] **Step 3: Update `BatchMessageResult` and handlers**

In `services/chorus-server/src/routes/batch.rs`, replace the existing `BatchMessageResult` (around lines 54-60) with:

```rust
/// One message result in the batch response.
#[derive(Serialize)]
pub struct BatchMessageResult {
    /// `Some` for queued, `None` for suppressed.
    pub message_id: Option<Uuid>,
    pub to: String,
    /// `"queued"` or `"suppressed"`.
    pub status: &'static str,
    /// Suppression reason when `status == "suppressed"`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}
```

Replace the entire body of `send_sms_batch` (currently lines 73-138) with:

```rust
pub async fn send_sms_batch(
    State(state): State<Arc<AppState>>,
    ctx: AccountContext,
    Json(req): Json<SendSmsBatchRequest>,
) -> Result<(StatusCode, Json<BatchSendResponse>), (StatusCode, String)> {
    validate_batch_size(req.recipients.len())?;

    let mut results = Vec::with_capacity(req.recipients.len());
    let mut any_suppressed = false;

    for recipient in &req.recipients {
        match crate::suppression::check_suppression(
            &state,
            ctx.account_id,
            "sms",
            &recipient.to,
        )
        .await
        {
            Err(crate::suppression::SuppressionRejection::Suppressed { reason }) => {
                any_suppressed = true;
                results.push(BatchMessageResult {
                    message_id: None,
                    to: recipient.to.clone(),
                    status: "suppressed",
                    reason: Some(reason),
                });
                continue;
            }
            Err(crate::suppression::SuppressionRejection::InvalidRecipient) => {
                return Err((
                    StatusCode::BAD_REQUEST,
                    format!("invalid recipient: {}", recipient.to),
                ));
            }
            Err(crate::suppression::SuppressionRejection::Db(e)) => {
                return Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string()));
            }
            Ok(()) => {}
        }

        let new_msg = NewMessage {
            account_id: ctx.account_id,
            api_key_id: ctx.key_id,
            channel: "sms".into(),
            sender: req.from.clone(),
            recipient: recipient.to.clone(),
            subject: None,
            body: recipient.body.clone(),
            environment: ctx.environment.clone(),
        };

        let message = match state.message_repo().insert(&new_msg).await {
            Ok(m) => m,
            Err(e) => {
                return Ok((
                    StatusCode::ACCEPTED,
                    Json(BatchSendResponse {
                        messages: results,
                        error: Some(format!("failed at recipient {}: {}", recipient.to, e)),
                    }),
                ));
            }
        };

        let job = SendJob {
            message_id: message.id,
            account_id: message.account_id,
            channel: "sms".into(),
            environment: message.environment.clone(),
            attempt: 0,
        };
        if let Err(e) = crate::queue::enqueue::notify(&state, &job).await {
            return Ok((
                StatusCode::ACCEPTED,
                Json(BatchSendResponse {
                    messages: results,
                    error: Some(format!("failed to enqueue for {}: {}", recipient.to, e)),
                }),
            ));
        }

        results.push(BatchMessageResult {
            message_id: Some(message.id),
            to: recipient.to.clone(),
            status: "queued",
            reason: None,
        });
    }

    let status = if any_suppressed {
        StatusCode::MULTI_STATUS
    } else {
        StatusCode::ACCEPTED
    };
    Ok((
        status,
        Json(BatchSendResponse {
            messages: results,
            error: None,
        }),
    ))
}
```

Apply the **identical structure** to `send_email_batch` — only differences are:
1. Channel literal `"email"` instead of `"sms"` (in the `check_suppression` call AND the `NewMessage::channel` AND the `SendJob::channel`).
2. `subject: Some(recipient.subject.clone())` instead of `subject: None`.
3. Request body type `SendEmailBatchRequest` and recipient field type `EmailBatchRecipient`.

- [ ] **Step 4: Run all tests**

Run: `cargo test -p chorus-server`

Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git add services/chorus-server/src/routes/batch.rs services/chorus-server/tests/api_test.rs
git commit -m "feat(server): per-entry suppression filter in batch routes"
```

---

### Task 14: Extend `/internal/bounces` to populate suppressions

**Files:**
- Modify: `services/chorus-server/src/routes/internal.rs`
- Modify: `services/chorus-server/tests/api_test.rs`

- [ ] **Step 1: Add a bounce-handling helper to `MessageRepository` (a transactional method)**

The bounce handler needs to look up a message by `provider_message_id` and execute three writes atomically. Add to `services/chorus-server/src/db/mod.rs::MessageRepository`:

```rust
    /// Find a message by its provider's message id (no account scoping — internal use only).
    async fn find_by_provider_message_id(
        &self,
        provider_message_id: &str,
    ) -> Result<Option<Message>, DbError>;
```

In `services/chorus-server/src/db/postgres.rs`, add the implementation alongside the other `MessageRepository` methods:

```rust
    async fn find_by_provider_message_id(
        &self,
        provider_message_id: &str,
    ) -> Result<Option<Message>, DbError> {
        let start = Instant::now();
        let msg = sqlx::query_as::<_, Message>(
            "SELECT * FROM messages WHERE provider_message_id = $1 LIMIT 1",
        )
        .bind(provider_message_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| DbError::Internal(e.into()))?;
        record_db_duration!("find_by_provider_message_id", start);
        Ok(msg)
    }
```

In `tests/api_test.rs::MockMessageRepo`, add to the `impl MessageRepository for MockMessageRepo` block:

```rust
    async fn find_by_provider_message_id(
        &self,
        _provider_message_id: &str,
    ) -> Result<Option<Message>, DbError> {
        Ok(None)
    }
```

(The mock returns `None` by default; tests that need a hit will inject a message via `messages` and look it up there. For the bounce-flow integration test, the test will use `MockMessageRepo`'s public state to seed a message before calling `/internal/bounces`.)

Actually — the existing `MockMessageRepo` exposes `messages: Mutex<Vec<Message>>`. We can read it. Implement the mock as:

```rust
    async fn find_by_provider_message_id(
        &self,
        provider_message_id: &str,
    ) -> Result<Option<Message>, DbError> {
        let msgs = self.messages.lock().unwrap();
        Ok(msgs
            .iter()
            .find(|m| m.provider_message_id.as_deref() == Some(provider_message_id))
            .cloned())
    }
```

- [ ] **Step 2: Write a failing bounce test**

Append to `tests/api_test.rs`:

```rust
#[tokio::test]
async fn bounce_creates_suppression_and_marks_message() {
    use std::env;
    env::set_var("BOUNCE_SECRET", "test-secret");

    let state = test_state();
    let app = create_router(Arc::clone(&state));

    // Seed a message with provider_message_id directly into the mock
    let account_id = state.account_repo()
        .find_by_api_key_hash(&hex::encode(Sha256::digest(TEST_API_KEY.as_bytes())))
        .await
        .unwrap()
        .unwrap()
        .0
        .id;

    // We'll send an email through the API to populate state.message_repo, then patch its provider_message_id.
    let _ = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/email/send")
                .header("authorization", format!("Bearer {TEST_API_KEY}"))
                .header("content-type", "application/json")
                .body(axum::body::Body::from(
                    r#"{"to":"bouncy@example.com","subject":"x","body":"y"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    // Reach into MockMessageRepo and stamp a provider_message_id on the inserted row.
    // (Since MockMessageRepo is local to the test file, expose a helper or use downcasting.
    // The simplest path: make MockMessageRepo expose `set_provider_message_id` directly
    // as a non-trait method, accessed via a typed handle in the test.)

    // ... see Step 3 for the helper added to MockMessageRepo ...

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/internal/bounces")
                .header("x-chorus-secret", "test-secret")
                .header("content-type", "application/json")
                .body(axum::body::Body::from(
                    r#"{"recipient":"bouncy@example.com","reason":"5.1.1 user unknown","message_id":"bounce-test-1"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);

    // Verify suppression was created
    // (test_state() already cloned the suppression_repo into AppState; we need to read it back —
    // expose a helper on test_state() or pass MockSuppressionRepo back to the test.)
}
```

This test reveals that `test_state()` returns just `Arc<AppState>` and gives the test no direct handle to the underlying mocks for verification. We need a richer test fixture. **Refactor the fixture in Step 3.**

- [ ] **Step 3: Refactor `test_state()` to return both the state and the mock handles**

In `tests/api_test.rs`, add:

```rust
struct TestFixture {
    state: Arc<AppState>,
    suppressions: Arc<MockSuppressionRepo>,
    messages: Arc<MockMessageRepo>,
    account_id: Uuid,
}

fn test_fixture() -> TestFixture {
    let key_hash = hex::encode(Sha256::digest(TEST_API_KEY.as_bytes()));
    let account_id = Uuid::new_v4();
    let key_id = Uuid::new_v4();

    let account_repo = Arc::new(MockAccountRepo {
        account: Account {
            id: account_id,
            name: "Test Account".into(),
            owner_email: "test@example.com".into(),
            is_active: true,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        },
        api_key: ApiKey {
            id: key_id,
            account_id,
            name: "test key".into(),
            key_prefix: "ch_test_abcdef12...".into(),
            environment: "test".into(),
            last_used_at: None,
            expires_at: None,
            is_revoked: false,
            created_at: Utc::now(),
        },
        key_hash,
    });

    let messages = Arc::new(MockMessageRepo::new());
    let suppressions = Arc::new(MockSuppressionRepo::new());
    let api_key_repo = Arc::new(MockApiKeyRepo);
    let provider_config_repo = Arc::new(MockProviderConfigRepo);
    let webhook_repo = Arc::new(MockWebhookRepo);

    let redis = redis::Client::open("redis://127.0.0.1:6379").unwrap();
    let config = Arc::new(Config::from_env());

    let state = Arc::new(AppState::with_repos(
        redis,
        config,
        account_repo,
        messages.clone(),
        api_key_repo,
        provider_config_repo,
        webhook_repo,
        suppressions.clone(),
    ));

    TestFixture { state, suppressions, messages, account_id }
}

// Keep the existing `test_state()` as a thin wrapper for tests that don't need the mocks.
fn test_state() -> Arc<AppState> {
    test_fixture().state
}
```

Add a helper to `MockMessageRepo` (after the `impl MessageRepository for MockMessageRepo` block):

```rust
impl MockMessageRepo {
    /// Set provider_message_id for the most recently inserted message — test helper.
    fn stamp_provider_message_id(&self, recipient: &str, pmid: &str) {
        let mut msgs = self.messages.lock().unwrap();
        if let Some(msg) = msgs.iter_mut().rev().find(|m| m.recipient == recipient) {
            msg.provider_message_id = Some(pmid.to_string());
        }
    }
}
```

And to `MockSuppressionRepo`:

```rust
impl MockSuppressionRepo {
    fn snapshot(&self) -> Vec<Suppression> {
        self.entries.lock().unwrap().clone()
    }
}
```

- [ ] **Step 4: Rewrite the bounce test using the fixture**

```rust
#[tokio::test]
async fn bounce_creates_suppression_and_marks_message_bounced() {
    std::env::set_var("BOUNCE_SECRET", "test-secret");

    let fx = test_fixture();
    let app = create_router(Arc::clone(&fx.state));

    // Send an email so a message row exists.
    let send = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/email/send")
                .header("authorization", format!("Bearer {TEST_API_KEY}"))
                .header("content-type", "application/json")
                .body(axum::body::Body::from(
                    r#"{"to":"bouncy@example.com","subject":"x","body":"y"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(send.status(), StatusCode::ACCEPTED);

    // Stamp provider_message_id so the bounce handler can find the message.
    fx.messages.stamp_provider_message_id("bouncy@example.com", "bounce-test-1");

    // POST the bounce.
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/internal/bounces")
                .header("x-chorus-secret", "test-secret")
                .header("content-type", "application/json")
                .body(axum::body::Body::from(
                    r#"{"recipient":"bouncy@example.com","reason":"5.1.1 user unknown","message_id":"bounce-test-1"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // Verify suppression created.
    let snapshot = fx.suppressions.snapshot();
    assert_eq!(snapshot.len(), 1);
    assert_eq!(snapshot[0].channel, "email");
    assert_eq!(snapshot[0].recipient, "bouncy@example.com");
    assert_eq!(snapshot[0].reason, "hard_bounce");
    assert_eq!(snapshot[0].source, "chorus-mail");
    assert_eq!(snapshot[0].account_id, fx.account_id);
}

#[tokio::test]
async fn bounce_with_unknown_message_id_returns_200_no_suppression() {
    std::env::set_var("BOUNCE_SECRET", "test-secret");

    let fx = test_fixture();
    let app = create_router(Arc::clone(&fx.state));

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/internal/bounces")
                .header("x-chorus-secret", "test-secret")
                .header("content-type", "application/json")
                .body(axum::body::Body::from(
                    r#"{"recipient":"x@example.com","reason":"5.1.1","message_id":"never-existed"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert!(fx.suppressions.snapshot().is_empty());
}
```

- [ ] **Step 5: Run the tests to confirm they fail**

Run: `cargo test -p chorus-server --test api_test bounce_creates_suppression_and_marks_message_bounced bounce_with_unknown_message_id_returns_200_no_suppression`

Expected: both FAIL — the bounce handler doesn't write suppressions yet.

- [ ] **Step 6: Update `handle_bounce` in `routes/internal.rs`**

Replace `pub async fn handle_bounce(...)` (lines 26-54) with:

```rust
/// Receive bounce notification from chorus-mail.
pub async fn handle_bounce(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<BounceNotification>,
) -> Result<StatusCode, (StatusCode, String)> {
    // Validate shared secret.
    let expected = state.config().bounce_secret.as_deref().unwrap_or("");
    let provided = headers
        .get("x-chorus-secret")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if expected.is_empty() || provided != expected {
        return Err((StatusCode::UNAUTHORIZED, "invalid secret".into()));
    }

    let pmid = body.message_id.trim_matches(|c| c == '<' || c == '>');

    let message = state
        .message_repo()
        .find_by_provider_message_id(pmid)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let Some(message) = message else {
        tracing::warn!(
            message_id = %pmid,
            recipient = %body.recipient,
            "bounce arrived for unknown provider_message_id; ignoring"
        );
        return Ok(StatusCode::OK);
    };

    // Use the recipient Chorus originally accepted (canonical), not the bounce envelope's
    // recipient (postfix may have rewritten via aliasing).
    let normalized = match crate::suppression::normalize(&message.channel, &message.recipient) {
        Ok(n) => n,
        Err(_) => {
            tracing::warn!(
                channel = %message.channel,
                recipient = %message.recipient,
                "could not normalize stored recipient — skipping suppression write"
            );
            return Ok(StatusCode::OK);
        }
    };

    // Write suppression (idempotent), update message status, append delivery event.
    state
        .suppression_repo()
        .add(&crate::db::NewSuppression {
            account_id: message.account_id,
            channel: message.channel.clone(),
            recipient: normalized,
            reason: "hard_bounce".into(),
            source: "chorus-mail".into(),
        })
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    state
        .message_repo()
        .update_status(message.id, "bounced", None, None, Some(&body.reason))
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    state
        .message_repo()
        .insert_delivery_event(
            message.id,
            "bounced",
            Some(serde_json::json!({
                "reason": body.reason,
                "source": "chorus-mail",
            })),
        )
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    tracing::info!(
        message_id = %message.id,
        account_id = %message.account_id,
        recipient = %body.recipient,
        "suppression added from chorus-mail bounce"
    );

    Ok(StatusCode::OK)
}
```

Note: this performs three sequential writes rather than a single transaction. The Postgres `MessageRepository` API doesn't expose a transaction interface, and adding one is out of scope here. The writes are individually idempotent (`ON CONFLICT DO NOTHING` for suppression; `update_status` is naturally idempotent; `insert_delivery_event` is the only write that could double on retry, which is acceptable for an audit log). If chorus-mail retries due to network error, the worst case is duplicate delivery_events — recoverable via the `created_at` timestamps.

- [ ] **Step 7: Run the bounce tests**

Run: `cargo test -p chorus-server --test api_test bounce_creates_suppression_and_marks_message_bounced bounce_with_unknown_message_id_returns_200_no_suppression`

Expected: PASS.

- [ ] **Step 8: Run all tests**

Run: `cargo test -p chorus-server`

Expected: all pass.

- [ ] **Step 9: Commit**

```bash
git add services/chorus-server/src/db/mod.rs services/chorus-server/src/db/postgres.rs services/chorus-server/src/routes/internal.rs services/chorus-server/tests/api_test.rs
git commit -m "feat(server): populate suppression list from chorus-mail bounces"
```

---

### Task 15: Final CI sweep

- [ ] **Step 1: cargo fmt**

Run: `cargo fmt --all`

Expected: no diff or only whitespace changes.

- [ ] **Step 2: cargo clippy**

Run: `cargo clippy --workspace --all-targets -- -D warnings`

Expected: no warnings.

- [ ] **Step 3: cargo test**

Run: `cargo test --workspace`

Expected: all pass.

- [ ] **Step 4: cargo deny**

Run: `cargo deny check`

Expected: advisories ok, bans ok, licenses ok, sources ok.

- [ ] **Step 5: Commit any formatting fixes**

```bash
git add -A
git diff --cached --quiet || git commit -m "style: cargo fmt"
```

---

## Summary

| Task | Component | Lines of Effort |
|---|---|---|
| 1 | Migration 007 | ~10 |
| 2 | Trait + types in db/mod.rs | ~50 |
| 3 | PgSuppressionRepository | ~80 |
| 4 | AppState wiring | ~40 |
| 5 | Test mock + fixture refactor | ~80 |
| 6 | normalize + check_suppression + unit tests | ~100 |
| 7 | /v1/suppressions routes | ~120 |
| 8 | /v1/suppressions integration tests | ~100 |
| 9 | /v1/sms/send hot path | ~60 |
| 10 | /v1/email/send hot path | ~60 |
| 11 | Refactor: centralize error response | ~30 |
| 12 | /v1/otp/send hot path | ~30 |
| 13 | Batch routes per-entry filter | ~80 |
| 14 | Extend /internal/bounces + bounce tests | ~150 |
| 15 | CI sweep | — |

Total ≈ 990 LoC of net change (production + tests). Estimated time for a focused engineer: **2-3 days**.

After this lands, follow-up specs can layer on:
- **C2.5:** External provider bounce webhooks (SES, Resend, Mailgun)
- **C3:** SMS STOP keyword detection
- **C1:** Idempotency keys (separate concern from suppression — different lifecycle, different storage)
