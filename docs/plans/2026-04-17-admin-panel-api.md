# Admin Panel API Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add admin-only REST API endpoints for account management, provider config, DLQ, message inspection, billing, and webhooks — enabling Strata dashboard to manage Chorus without direct DB access.

**Architecture:** New `AdminContext` extractor authenticated by `ch_admin_` API keys stored in a dedicated `admin_keys` table. All admin routes live under `/admin/` prefix with a shared admin auth middleware layer. Each feature module adds repository trait methods + route handlers following existing patterns.

**Tech Stack:** Rust, Axum, sqlx, Redis, serde, uuid, chrono

**Closes:** #34, #35, #36, #37, #38, #39

---

### Task 1: Create Admin Auth Migration

**Files:**
- Create: `services/chorus-server/src/db/migrations/005_create_admin_keys.sql`

**Step 1: Write migration**

```sql
CREATE TABLE admin_keys (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name TEXT NOT NULL,
    key_hash TEXT NOT NULL UNIQUE,
    key_prefix TEXT NOT NULL,
    is_revoked BOOLEAN NOT NULL DEFAULT false,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_admin_keys_key_hash ON admin_keys (key_hash);
```

**Step 2: Run `cargo check -p chorus-server`**

Expected: compiles (migration is SQL, not compiled, but verify no syntax issues at startup later).

**Step 3: Commit**

```bash
git add services/chorus-server/src/db/migrations/005_create_admin_keys.sql
git commit -m "feat(server): add admin_keys migration"
```

---

### Task 2: Create AdminContext Extractor

**Files:**
- Create: `services/chorus-server/src/auth/admin.rs`
- Modify: `services/chorus-server/src/auth/mod.rs`

**Step 1: Create admin auth extractor**

Create `services/chorus-server/src/auth/admin.rs`:
```rust
use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::http::StatusCode;
use sha2::{Digest, Sha256};
use std::sync::Arc;
use uuid::Uuid;

use crate::app::AppState;

/// Authenticated admin context extracted from an admin API key.
#[derive(Debug, Clone)]
pub struct AdminContext {
    /// The admin key ID used to authenticate.
    pub key_id: Uuid,
}

impl FromRequestParts<Arc<AppState>> for AdminContext {
    type Rejection = (StatusCode, &'static str);

    async fn from_request_parts(
        parts: &mut Parts,
        state: &Arc<AppState>,
    ) -> Result<Self, Self::Rejection> {
        let header = parts
            .headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .ok_or((StatusCode::UNAUTHORIZED, "missing authorization header"))?;

        let key = header
            .strip_prefix("Bearer ")
            .ok_or((StatusCode::UNAUTHORIZED, "invalid authorization format"))?;

        if !key.starts_with("ch_admin_") {
            return Err((StatusCode::UNAUTHORIZED, "invalid admin key format"));
        }

        let hash = hex::encode(Sha256::digest(key.as_bytes()));

        let admin_key = state
            .admin_key_repo()
            .find_by_hash(&hash)
            .await
            .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "database error"))?
            .ok_or((StatusCode::UNAUTHORIZED, "invalid admin key"))?;

        if admin_key.is_revoked {
            return Err((StatusCode::UNAUTHORIZED, "admin key is revoked"));
        }

        Ok(AdminContext {
            key_id: admin_key.id,
        })
    }
}
```

**Step 2: Export admin module**

In `services/chorus-server/src/auth/mod.rs`, add:
```rust
pub mod admin;
```

**Step 3: Run `cargo check -p chorus-server`**

Expected: will fail — `admin_key_repo()` and `AdminKey` type don't exist yet. That's expected; we'll fix in next tasks.

**Step 4: Commit**

```bash
git add services/chorus-server/src/auth/
git commit -m "feat(server): add AdminContext extractor (WIP — needs repo)"
```

---

### Task 3: Add AdminKey Type + Repository Trait

**Files:**
- Modify: `services/chorus-server/src/db/mod.rs`
- Modify: `services/chorus-server/src/db/postgres.rs`
- Modify: `services/chorus-server/src/app.rs`

**Step 1: Add AdminKey type and trait to db/mod.rs**

Add after existing types:
```rust
/// An admin API key for dashboard access.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct AdminKey {
    pub id: Uuid,
    pub name: String,
    pub key_prefix: String,
    pub is_revoked: bool,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// Repository for admin key operations.
#[async_trait::async_trait]
pub trait AdminKeyRepository: Send + Sync {
    /// Find an admin key by its SHA-256 hash.
    async fn find_by_hash(&self, hash: &str) -> Result<Option<AdminKey>, DbError>;
}
```

**Step 2: Implement in postgres.rs**

Add at the end of `postgres.rs`:
```rust
#[async_trait]
impl AdminKeyRepository for PgRepository {
    async fn find_by_hash(&self, hash: &str) -> Result<Option<AdminKey>, DbError> {
        let key = sqlx::query_as::<_, AdminKey>(
            "SELECT id, name, key_prefix, is_revoked, created_at
             FROM admin_keys WHERE key_hash = $1",
        )
        .bind(hash)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| DbError::Internal(e.into()))?;

        Ok(key)
    }
}
```

**Step 3: Add admin_key_repo() to AppState**

In `app.rs`, add a method to `AppState` and the necessary field. Follow the existing pattern used by `account_repo()`, `message_repo()`, etc.

Add to `AppState` struct:
```rust
admin_key_repo: Arc<dyn AdminKeyRepository>,
```

Add getter:
```rust
pub fn admin_key_repo(&self) -> Arc<dyn AdminKeyRepository> {
    Arc::clone(&self.admin_key_repo)
}
```

Initialize in `AppState::new()` (or wherever AppState is constructed):
```rust
admin_key_repo: Arc::new(PgRepository::new(pool.clone())),
```

**Step 4: Run `cargo check -p chorus-server`**

Expected: compiles cleanly (AdminContext extractor can now resolve).

**Step 5: Commit**

```bash
git add services/chorus-server/src/db/ services/chorus-server/src/app.rs
git commit -m "feat(server): add AdminKey type, repository trait, and AppState wiring"
```

---

### Task 4: Create Admin Routes Module + Wire into Router

**Files:**
- Create: `services/chorus-server/src/routes/admin/mod.rs`
- Create: `services/chorus-server/src/routes/admin/accounts.rs`
- Modify: `services/chorus-server/src/routes/mod.rs`
- Modify: `services/chorus-server/src/app.rs`

**Step 1: Create admin routes module**

Create `services/chorus-server/src/routes/admin/mod.rs`:
```rust
pub mod accounts;

use axum::routing::{delete, get, patch, post};
use axum::Router;
use std::sync::Arc;

use crate::app::AppState;

/// Build the admin sub-router with all admin endpoints.
pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        // Account Management (#34)
        .route("/accounts", get(accounts::list))
        .route("/accounts/{id}", get(accounts::detail))
        .route("/accounts", post(accounts::create))
        .route("/accounts/{id}", patch(accounts::update))
        .route("/accounts/{id}", delete(accounts::soft_delete))
}
```

**Step 2: Create placeholder accounts handler**

Create `services/chorus-server/src/routes/admin/accounts.rs`:
```rust
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use crate::app::AppState;
use crate::auth::admin::AdminContext;

#[derive(Serialize)]
pub struct AccountListItem {
    pub id: Uuid,
    pub name: String,
    pub owner_email: String,
    pub is_active: bool,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// `GET /admin/accounts` — list all accounts.
pub async fn list(
    State(state): State<Arc<AppState>>,
    _admin: AdminContext,
) -> Result<Json<Vec<AccountListItem>>, (StatusCode, String)> {
    let accounts = state
        .admin_repo()
        .list_accounts()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(accounts))
}

#[derive(Serialize)]
pub struct AccountDetail {
    pub id: Uuid,
    pub name: String,
    pub owner_email: String,
    pub is_active: bool,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    pub key_count: i64,
    pub message_count: i64,
}

/// `GET /admin/accounts/{id}` — account detail with usage stats.
pub async fn detail(
    State(state): State<Arc<AppState>>,
    _admin: AdminContext,
    Path(id): Path<Uuid>,
) -> Result<Json<AccountDetail>, (StatusCode, String)> {
    let account = state
        .admin_repo()
        .get_account_detail(id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "account not found".into()))?;

    Ok(Json(account))
}

#[derive(Deserialize)]
pub struct CreateAccountRequest {
    pub name: String,
    pub owner_email: String,
}

/// `POST /admin/accounts` — create a new account.
pub async fn create(
    State(state): State<Arc<AppState>>,
    _admin: AdminContext,
    Json(body): Json<CreateAccountRequest>,
) -> Result<(StatusCode, Json<AccountListItem>), (StatusCode, String)> {
    let account = state
        .admin_repo()
        .create_account(&body.name, &body.owner_email)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok((StatusCode::CREATED, Json(account)))
}

#[derive(Deserialize)]
pub struct UpdateAccountRequest {
    pub is_active: Option<bool>,
    pub name: Option<String>,
}

/// `PATCH /admin/accounts/{id}` — update account fields.
pub async fn update(
    State(state): State<Arc<AppState>>,
    _admin: AdminContext,
    Path(id): Path<Uuid>,
    Json(body): Json<UpdateAccountRequest>,
) -> Result<StatusCode, (StatusCode, String)> {
    state
        .admin_repo()
        .update_account(id, body.is_active, body.name.as_deref())
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(StatusCode::NO_CONTENT)
}

/// `DELETE /admin/accounts/{id}` — soft-delete account.
pub async fn soft_delete(
    State(state): State<Arc<AppState>>,
    _admin: AdminContext,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, (StatusCode, String)> {
    state
        .admin_repo()
        .deactivate_account(id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(StatusCode::NO_CONTENT)
}
```

**Step 3: Export admin module in routes/mod.rs**

Add to `services/chorus-server/src/routes/mod.rs`:
```rust
pub mod admin;
```

**Step 4: Wire admin router into app.rs**

In `create_router_with_metrics`, after the main router `.with_state(state)` call, add the admin sub-router *before* the metrics middleware layer:
```rust
    let mut router = Router::new()
        // ... all existing routes ...
        .nest("/admin", routes::admin::router())
        .with_state(state)
        .layer(axum_middleware::from_fn(crate::middleware::metrics::track));
```

**Step 5: Run `cargo check -p chorus-server`**

Expected: will fail — `admin_repo()` doesn't exist yet. Commit WIP.

**Step 6: Commit**

```bash
git add services/chorus-server/src/routes/ services/chorus-server/src/app.rs
git commit -m "feat(server): add admin routes module with account endpoints (WIP — needs repo)"
```

---

### Task 5: Create AdminRepository Trait + Implementation (#34)

**Files:**
- Create: `services/chorus-server/src/db/admin.rs`
- Modify: `services/chorus-server/src/db/mod.rs`
- Modify: `services/chorus-server/src/app.rs`

**Step 1: Create AdminRepository trait and Pg implementation**

Create `services/chorus-server/src/db/admin.rs`:
```rust
use async_trait::async_trait;
use sqlx::PgPool;
use std::time::Instant;
use uuid::Uuid;

use super::DbError;
use crate::routes::admin::accounts::{AccountDetail, AccountListItem};

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
}

/// PostgreSQL implementation.
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
            "SELECT id, name, owner_email, is_active, created_at FROM accounts ORDER BY created_at DESC",
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
}
```

**Step 2: Export in db/mod.rs**

Add:
```rust
pub mod admin;
pub use admin::{AdminRepository, PgAdminRepository};
```

**Step 3: Add admin_repo() to AppState**

Add field, getter, and initialization following the same pattern as other repos.

**Step 4: Run `cargo check -p chorus-server`**

Expected: compiles cleanly. Note: `AccountListItem` and `AccountDetail` need `sqlx::FromRow` derive added in accounts.rs.

**Step 5: Run `cargo test -p chorus-server`**

Expected: all existing tests pass.

**Step 6: Commit**

```bash
git add services/chorus-server/src/db/ services/chorus-server/src/app.rs services/chorus-server/src/routes/admin/
git commit -m "feat(server): add AdminRepository + account management endpoints (closes #34)"
```

---

### Task 6: Admin Provider Config Endpoints (#35)

**Files:**
- Create: `services/chorus-server/src/routes/admin/providers.rs`
- Modify: `services/chorus-server/src/routes/admin/mod.rs`
- Modify: `services/chorus-server/src/db/admin.rs`

**Step 1: Add provider admin methods to AdminRepository trait**

Add to trait in `db/admin.rs`:
```rust
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
```

**Step 2: Define response types**

In `routes/admin/providers.rs`:
```rust
#[derive(Serialize, sqlx::FromRow)]
pub struct AdminProviderConfig {
    pub id: Uuid,
    pub account_id: Uuid,
    pub channel: String,
    pub provider: String,
    pub priority: i32,
    pub is_active: bool,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Serialize)]
pub struct ProviderHealth {
    pub id: Uuid,
    pub provider: String,
    pub total_sent: i64,
    pub total_errors: i64,
    pub error_rate: f64,
    pub last_success: Option<chrono::DateTime<chrono::Utc>>,
    pub last_error: Option<chrono::DateTime<chrono::Utc>>,
}
```

**Step 3: Implement handlers**

Endpoints:
- `GET /admin/providers` — list_all
- `GET /admin/providers/{id}/health` — health
- `PATCH /admin/providers/{id}` — update
- `POST /admin/providers/disable` — bulk disable by provider name

**Step 4: Implement Pg queries**

Health query joins `messages` + `delivery_events` tables to compute error rate:
```sql
SELECT pc.id, pc.provider,
       COUNT(CASE WHEN m.status = 'delivered' THEN 1 END) AS total_sent,
       COUNT(CASE WHEN m.status = 'failed' THEN 1 END) AS total_errors,
       MAX(CASE WHEN m.status = 'delivered' THEN m.delivered_at END) AS last_success,
       MAX(CASE WHEN de.status = 'failed_attempt' THEN de.created_at END) AS last_error
FROM provider_configs pc
LEFT JOIN messages m ON m.provider = pc.provider AND m.account_id = pc.account_id
LEFT JOIN delivery_events de ON de.message_id = m.id
WHERE pc.id = $1
GROUP BY pc.id, pc.provider
```

**Step 5: Wire routes**

In `routes/admin/mod.rs`, add:
```rust
pub mod providers;
```

Add routes:
```rust
    .route("/providers", get(providers::list_all))
    .route("/providers/{id}/health", get(providers::health))
    .route("/providers/{id}", patch(providers::update))
    .route("/providers/disable", post(providers::bulk_disable))
```

**Step 6: Run `cargo check -p chorus-server`**

**Step 7: Commit**

```bash
git add services/chorus-server/src/
git commit -m "feat(server): add admin provider config + health endpoints (closes #35)"
```

---

### Task 7: Admin DLQ Management Endpoints (#36)

**Files:**
- Create: `services/chorus-server/src/routes/admin/dlq.rs`
- Modify: `services/chorus-server/src/routes/admin/mod.rs`
- Modify: `services/chorus-server/src/db/admin.rs`

**Step 1: Add DLQ methods to AdminRepository**

DLQ lives in Redis (`chorus:dead_letters` list). We need methods that:
1. Read DLQ entries from Redis (LRANGE)
2. Look up message details from PostgreSQL
3. Re-enqueue by removing from DLQ and pushing to main queue

Add to trait:
```rust
    /// List DLQ messages with pagination.
    async fn list_dlq_messages(
        &self,
        limit: i64,
        offset: i64,
        channel: Option<&str>,
        account_id: Option<Uuid>,
    ) -> Result<Vec<DlqMessage>, DbError>;
    /// Get DLQ message detail with full retry history.
    async fn get_dlq_message_detail(&self, message_id: Uuid) -> Result<Option<DlqMessageDetail>, DbError>;
```

**Step 2: DLQ Redis operations**

Create helper functions in a new file or in `routes/admin/dlq.rs` for:
- `list_dlq` — LRANGE on `chorus:dead_letters`, parse JSON, join with DB
- `retry_single` — LREM from DLQ, reset attempt to 0, LPUSH to main queue
- `retry_batch` — same for multiple message IDs
- `purge_single` — LREM from DLQ
- `purge_all` — DEL the entire DLQ key (with date filter = scan + LREM)

**Step 3: Response types**

```rust
#[derive(Serialize)]
pub struct DlqMessage {
    pub message_id: Uuid,
    pub account_id: Uuid,
    pub channel: String,
    pub recipient: String,
    pub status: String,
    pub attempts: i32,
    pub error_message: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Serialize)]
pub struct DlqMessageDetail {
    pub message: DlqMessage,
    pub delivery_events: Vec<crate::db::DeliveryEvent>,
}
```

**Step 4: Implement handlers**

Endpoints:
- `GET /admin/dlq` — list (query params: limit, offset, channel, account_id)
- `GET /admin/dlq/{message_id}` — detail with retry history
- `POST /admin/dlq/{message_id}/retry` — re-enqueue single
- `POST /admin/dlq/retry-batch` — re-enqueue multiple (JSON body with message_ids)
- `DELETE /admin/dlq/{message_id}` — purge single
- `DELETE /admin/dlq/purge` — purge old (query param: older_than_days)

**Step 5: Wire routes**

```rust
pub mod dlq;
```

```rust
    .route("/dlq", get(dlq::list))
    .route("/dlq/{message_id}", get(dlq::detail))
    .route("/dlq/{message_id}/retry", post(dlq::retry_single))
    .route("/dlq/retry-batch", post(dlq::retry_batch))
    .route("/dlq/{message_id}", delete(dlq::purge_single))
    .route("/dlq/purge", delete(dlq::purge_all))
```

**Step 6: Run `cargo check -p chorus-server` + `cargo test -p chorus-server`**

**Step 7: Commit**

```bash
git add services/chorus-server/src/
git commit -m "feat(server): add admin DLQ management endpoints (closes #36)"
```

---

### Task 8: Admin Message Inspector Endpoints (#37)

**Files:**
- Create: `services/chorus-server/src/routes/admin/messages.rs`
- Modify: `services/chorus-server/src/routes/admin/mod.rs`
- Modify: `services/chorus-server/src/db/admin.rs`

**Step 1: Add message search to AdminRepository**

```rust
    /// Search messages across all accounts with filters.
    async fn search_messages(&self, filters: &MessageSearchFilters) -> Result<Vec<Message>, DbError>;
    /// Get message detail with delivery timeline.
    async fn get_message_detail(&self, id: Uuid) -> Result<Option<MessageDetail>, DbError>;
```

**Step 2: Define filter struct**

```rust
#[derive(Deserialize)]
pub struct MessageSearchFilters {
    pub account_id: Option<Uuid>,
    pub channel: Option<String>,
    pub status: Option<String>,
    pub provider: Option<String>,
    pub date_from: Option<chrono::DateTime<chrono::Utc>>,
    pub date_to: Option<chrono::DateTime<chrono::Utc>>,
    pub recipient: Option<String>,       // partial match with ILIKE
    pub min_cost: Option<i64>,           // microdollars
    pub max_cost: Option<i64>,
    pub limit: Option<i64>,              // default 50
    pub offset: Option<i64>,             // default 0
}
```

**Step 3: Implement dynamic SQL query**

Build query with optional WHERE clauses. Use sqlx's query builder or manual string building with bind parameters:

```rust
async fn search_messages(&self, f: &MessageSearchFilters) -> Result<Vec<Message>, DbError> {
    let mut query = String::from("SELECT * FROM messages WHERE 1=1");
    // Dynamically append conditions based on which filters are Some
    // Use positional parameters ($1, $2, etc.)
    // ORDER BY created_at DESC LIMIT $N OFFSET $M
}
```

**Step 4: Message detail includes delivery events**

```rust
#[derive(Serialize)]
pub struct MessageDetail {
    pub message: Message,
    pub delivery_events: Vec<DeliveryEvent>,
}
```

Query: fetch message (no account_id filter) + delivery events.

**Step 5: Handlers**

- `GET /admin/messages` — search (all filters as query params)
- `GET /admin/messages/{id}` — detail with timeline

**Step 6: Wire routes**

```rust
pub mod messages;
```

```rust
    .route("/messages", get(messages::search))
    .route("/messages/{id}", get(messages::detail))
```

**Step 7: Run `cargo check -p chorus-server`**

**Step 8: Commit**

```bash
git add services/chorus-server/src/
git commit -m "feat(server): add admin message inspector endpoints (closes #37)"
```

---

### Task 9: Admin Billing Endpoints (#38)

**Files:**
- Create: `services/chorus-server/src/routes/admin/billing.rs`
- Modify: `services/chorus-server/src/routes/admin/mod.rs`
- Modify: `services/chorus-server/src/db/admin.rs`

**Step 1: Add billing admin methods to AdminRepository**

```rust
    /// List all accounts with billing status.
    async fn list_billing_accounts(&self) -> Result<Vec<BillingAccountSummary>, DbError>;
    /// Override an account's subscription plan.
    async fn override_plan(&self, account_id: Uuid, plan_slug: &str) -> Result<(), DbError>;
    /// Adjust usage counters.
    async fn adjust_usage(
        &self,
        account_id: Uuid,
        sms_delta: Option<i32>,
        email_delta: Option<i32>,
    ) -> Result<(), DbError>;
    /// Generate billing report.
    async fn billing_report(&self) -> Result<BillingReport, DbError>;
```

**Step 2: Response types**

```rust
#[derive(Serialize)]
pub struct BillingAccountSummary {
    pub account_id: Uuid,
    pub account_name: String,
    pub plan_slug: String,
    pub status: String,
    pub sms_sent: i32,
    pub sms_quota: i32,
    pub email_sent: i32,
    pub email_quota: i32,
    pub period_end: chrono::DateTime<chrono::Utc>,
}

#[derive(Serialize)]
pub struct BillingReport {
    pub total_revenue_cents: i64,
    pub accounts_by_plan: Vec<PlanCount>,
    pub overage_accounts: Vec<Uuid>,
}

#[derive(Serialize)]
pub struct PlanCount {
    pub plan_slug: String,
    pub count: i64,
}
```

**Step 3: Handlers**

- `GET /admin/billing/accounts` — list with subscription + usage
- `PATCH /admin/billing/accounts/{id}/plan` — override plan
- `PATCH /admin/billing/accounts/{id}/usage` — adjust usage
- `GET /admin/billing/reports` — billing report

**Step 4: Wire routes + commit**

```bash
git add services/chorus-server/src/
git commit -m "feat(server): add admin billing endpoints (closes #38)"
```

---

### Task 10: Admin Webhook Endpoints (#39)

**Files:**
- Create: `services/chorus-server/src/routes/admin/webhooks.rs`
- Create: `services/chorus-server/src/db/migrations/006_webhook_deliveries.sql`
- Modify: `services/chorus-server/src/routes/admin/mod.rs`
- Modify: `services/chorus-server/src/db/admin.rs`

**Step 1: Create webhook_deliveries table migration**

Currently webhook delivery logs are not persisted (only retried via Redis). We need a table to store delivery history for the admin panel.

```sql
CREATE TABLE webhook_deliveries (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    webhook_id UUID NOT NULL REFERENCES webhooks(id),
    event TEXT NOT NULL,
    payload JSONB NOT NULL,
    response_status INTEGER,
    response_body TEXT,
    attempt INTEGER NOT NULL DEFAULT 1,
    success BOOLEAN NOT NULL DEFAULT false,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_webhook_deliveries_webhook_id ON webhook_deliveries (webhook_id);
CREATE INDEX idx_webhook_deliveries_created_at ON webhook_deliveries (created_at DESC);
```

**Step 2: Add webhook admin methods to AdminRepository**

```rust
    /// List all webhooks across all accounts.
    async fn list_all_webhooks(&self) -> Result<Vec<AdminWebhook>, DbError>;
    /// Get webhook delivery log.
    async fn get_webhook_deliveries(
        &self,
        webhook_id: Uuid,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<WebhookDelivery>, DbError>;
    /// Enable/disable a webhook.
    async fn update_webhook_status(&self, id: Uuid, is_active: bool) -> Result<(), DbError>;
    /// Disable all webhooks for an account.
    async fn disable_account_webhooks(&self, account_id: Uuid) -> Result<u64, DbError>;
```

**Step 3: Test delivery endpoint**

`POST /admin/webhooks/{id}/test` sends a test payload to the webhook URL using the existing `deliver_webhook()` function from `webhook_dispatch.rs`.

**Step 4: Handlers**

- `GET /admin/webhooks` — list all
- `GET /admin/webhooks/{id}/deliveries` — delivery log
- `POST /admin/webhooks/{id}/test` — send test event
- `PATCH /admin/webhooks/{id}` — enable/disable

**Step 5: Update webhook_dispatch.rs**

Modify `deliver_webhook()` to also INSERT into `webhook_deliveries` table for audit trail. This requires passing a `PgPool` or repo reference to the dispatch function.

**Step 6: Wire routes + commit**

```bash
git add services/chorus-server/src/ services/chorus-server/src/db/migrations/
git commit -m "feat(server): add admin webhook endpoints + delivery log (closes #39)"
```

---

### Task 11: Add Integration Tests for Admin Auth

**Files:**
- Modify: `services/chorus-server/tests/api_test.rs`

**Step 1: Add admin auth tests**

```rust
#[tokio::test]
async fn admin_accounts_without_auth_returns_401() {
    let app = create_test_app().await;
    let response = app.get("/admin/accounts").send().await;
    assert_eq!(response.status(), 401);
}

#[tokio::test]
async fn admin_accounts_with_user_key_returns_401() {
    let app = create_test_app().await;
    let response = app
        .get("/admin/accounts")
        .header("authorization", "Bearer ch_live_test123")
        .send()
        .await;
    assert_eq!(response.status(), 401);
}

#[tokio::test]
async fn admin_accounts_with_admin_key_returns_200() {
    let app = create_test_app_with_admin_key().await;
    let response = app
        .get("/admin/accounts")
        .header("authorization", "Bearer ch_admin_testkey")
        .send()
        .await;
    assert_eq!(response.status(), 200);
}
```

**Step 2: Run tests**

```bash
cargo test -p chorus-server
```

**Step 3: Commit**

```bash
git add services/chorus-server/tests/
git commit -m "test(server): add admin auth integration tests"
```

---

### Task 12: Final Verification

**Step 1: Run full CI checks**

```bash
cargo test --workspace
cargo clippy --workspace -- -D warnings
cargo fmt --all
cargo deny check
```

**Step 2: Commit any formatting fixes**

```bash
git add -A && git commit -m "style: format admin panel code"
```

(Skip if no changes.)

---

## Summary

| Task | Issue | Scope |
|------|-------|-------|
| 1-4 | Foundation | Admin auth migration, extractor, repo, route structure |
| 5 | #34 | Account management (CRUD + usage stats) |
| 6 | #35 | Provider config dashboard + health |
| 7 | #36 | DLQ management (Redis + DB hybrid) |
| 8 | #37 | Message inspector (cross-account search) |
| 9 | #38 | Billing administration |
| 10 | #39 | Webhook admin + delivery log table |
| 11-12 | All | Integration tests + final verification |
