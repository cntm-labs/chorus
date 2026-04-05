# chorus-server Phase 2a Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Build the Axum REST API server with PostgreSQL, Redis queue, API key auth, sending endpoints, message history, OTP, and Docker Compose dev environment.

**Architecture:** Axum server with repository trait abstraction (sqlx now, sentinel later). Async message queue via Redis. OTP stored in Redis with TTL. All business logic delegates to chorus-core for actual delivery.

**Tech Stack:** Rust, Axum 0.8, sqlx 0.8, redis-rs 0.27, PostgreSQL 16, Redis 7, Docker Compose

**Execution order:** Scaffold → Config → DB/Migrations → Auth middleware → Health endpoint → Queue → Send endpoints → OTP → Messages → API Keys → Docker Compose → Verify

---

## Task 1: Scaffold chorus-server Crate

### Files
- Create: `crates/chorus-server/Cargo.toml`
- Create: `crates/chorus-server/src/main.rs`
- Create: `crates/chorus-server/src/config.rs`
- Create: `crates/chorus-server/src/app.rs`
- Modify: `Cargo.toml` (workspace members)

### Step 1: Add workspace dependencies

Add to root `Cargo.toml` under `[workspace.dependencies]`:

```toml
# Web
axum = "0.8"
tower-http = { version = "0.6", features = ["cors", "trace"] }

# Database
sqlx = { version = "0.8", features = ["runtime-tokio", "tls-native-tls", "postgres", "uuid", "chrono", "json"] }

# Redis
redis = { version = "0.27", features = ["tokio-comp"] }

# Crypto
sha2 = "0.10"

# Config
dotenvy = "0.15"
```

Add `"crates/chorus-server"` to `workspace.members`.

### Step 2: Create crate Cargo.toml

```toml
[package]
name = "chorus-server"
version = "0.1.1"
edition = "2021"
license = "MIT"
description = "Axum REST API server for Chorus CPaaS"

[dependencies]
chorus-core = { path = "../chorus-core" }
chorus-providers = { path = "../chorus-providers" }
axum = { workspace = true }
tower-http = { workspace = true }
tokio = { workspace = true }
sqlx = { workspace = true }
redis = { workspace = true }
serde = { workspace = true }
serde_json = { workspace = true }
tracing = { workspace = true }
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
uuid = { workspace = true }
chrono = { workspace = true }
sha2 = { workspace = true }
anyhow = { workspace = true }
thiserror = { workspace = true }
dotenvy = { workspace = true }
rand = { workspace = true }
hex = { workspace = true }

[lints]
workspace = true
```

### Step 3: Create config.rs

```rust
use std::env;

pub struct Config {
    pub database_url: String,
    pub redis_url: String,
    pub host: String,
    pub port: u16,
}

impl Config {
    pub fn from_env() -> Self {
        Self {
            database_url: env::var("DATABASE_URL")
                .unwrap_or_else(|_| "postgres://chorus:chorus@localhost:5432/chorus".into()),
            redis_url: env::var("REDIS_URL")
                .unwrap_or_else(|_| "redis://127.0.0.1:6379".into()),
            host: env::var("HOST").unwrap_or_else(|_| "0.0.0.0".into()),
            port: env::var("PORT")
                .ok()
                .and_then(|p| p.parse().ok())
                .unwrap_or(3000),
        }
    }
}
```

### Step 4: Create app.rs

```rust
use axum::Router;
use sqlx::PgPool;

pub struct AppState {
    pub db: PgPool,
    pub redis: redis::Client,
}

pub fn create_router(_state: AppState) -> Router {
    Router::new()
}
```

### Step 5: Create main.rs

```rust
mod app;
mod config;

use config::Config;
use app::{AppState, create_router};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "chorus_server=debug,tower_http=debug".into()),
        )
        .init();

    let config = Config::from_env();

    let db = sqlx::PgPool::connect(&config.database_url).await?;
    sqlx::migrate!("src/db/migrations").run(&db).await?;

    let redis = redis::Client::open(config.redis_url.as_str())?;

    let state = AppState { db, redis };
    let app = create_router(state);

    let addr = format!("{}:{}", config.host, config.port);
    tracing::info!("chorus-server listening on {}", addr);

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
```

### Step 6: Verify compilation

Run: `cargo check --workspace`
Expected: Compiles with no errors (will warn about unused imports — that's OK for now)

### Step 7: Commit

```bash
git add Cargo.toml crates/chorus-server/
git commit -m "feat(server): scaffold chorus-server crate with Axum, sqlx, redis"
```

---

## Task 2: Database Migrations & Repository Traits

### Files
- Create: `crates/chorus-server/src/db/mod.rs`
- Create: `crates/chorus-server/src/db/postgres.rs`
- Create: `crates/chorus-server/src/db/migrations/001_initial.sql`

### Step 1: Create migration SQL

File: `crates/chorus-server/src/db/migrations/001_initial.sql`

```sql
CREATE TABLE accounts (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name TEXT NOT NULL,
    owner_email TEXT NOT NULL UNIQUE,
    is_active BOOLEAN NOT NULL DEFAULT true,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE api_keys (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    account_id UUID NOT NULL REFERENCES accounts(id),
    name TEXT NOT NULL,
    key_hash TEXT NOT NULL UNIQUE,
    key_prefix TEXT NOT NULL,
    environment TEXT NOT NULL CHECK (environment IN ('live', 'test')),
    last_used_at TIMESTAMPTZ,
    expires_at TIMESTAMPTZ,
    is_revoked BOOLEAN NOT NULL DEFAULT false,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_api_keys_key_hash ON api_keys(key_hash);
CREATE INDEX idx_api_keys_account_id ON api_keys(account_id);

CREATE TABLE messages (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    account_id UUID NOT NULL REFERENCES accounts(id),
    api_key_id UUID NOT NULL REFERENCES api_keys(id),
    channel TEXT NOT NULL CHECK (channel IN ('sms', 'email')),
    provider TEXT,
    sender TEXT,
    recipient TEXT NOT NULL,
    subject TEXT,
    body TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'queued',
    provider_message_id TEXT,
    error_message TEXT,
    cost_microdollars BIGINT NOT NULL DEFAULT 0,
    attempts INTEGER NOT NULL DEFAULT 0,
    environment TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    delivered_at TIMESTAMPTZ
);

CREATE INDEX idx_messages_account_id ON messages(account_id);
CREATE INDEX idx_messages_created_at ON messages(created_at DESC);

CREATE TABLE delivery_events (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    message_id UUID NOT NULL REFERENCES messages(id),
    status TEXT NOT NULL,
    provider_data JSONB,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_delivery_events_message_id ON delivery_events(message_id);
```

### Step 2: Create repository traits (db/mod.rs)

```rust
pub mod postgres;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum DbError {
    #[error("not found")]
    NotFound,
    #[error("database error: {0}")]
    Internal(#[from] anyhow::Error),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Account {
    pub id: Uuid,
    pub name: String,
    pub owner_email: String,
    pub is_active: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiKey {
    pub id: Uuid,
    pub account_id: Uuid,
    pub name: String,
    pub key_prefix: String,
    pub environment: String,
    pub last_used_at: Option<DateTime<Utc>>,
    pub expires_at: Option<DateTime<Utc>>,
    pub is_revoked: bool,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub id: Uuid,
    pub account_id: Uuid,
    pub api_key_id: Uuid,
    pub channel: String,
    pub provider: Option<String>,
    pub sender: Option<String>,
    pub recipient: String,
    pub subject: Option<String>,
    pub body: String,
    pub status: String,
    pub provider_message_id: Option<String>,
    pub error_message: Option<String>,
    pub cost_microdollars: i64,
    pub attempts: i32,
    pub environment: String,
    pub created_at: DateTime<Utc>,
    pub delivered_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeliveryEvent {
    pub id: Uuid,
    pub message_id: Uuid,
    pub status: String,
    pub provider_data: Option<serde_json::Value>,
    pub created_at: DateTime<Utc>,
}

pub struct NewMessage {
    pub account_id: Uuid,
    pub api_key_id: Uuid,
    pub channel: String,
    pub sender: Option<String>,
    pub recipient: String,
    pub subject: Option<String>,
    pub body: String,
    pub environment: String,
}

pub struct Pagination {
    pub limit: i64,
    pub offset: i64,
}

#[async_trait]
pub trait AccountRepository: Send + Sync {
    async fn find_by_api_key_hash(&self, hash: &str) -> Result<Option<(Account, ApiKey)>, DbError>;
    async fn update_key_last_used(&self, key_id: Uuid) -> Result<(), DbError>;
}

#[async_trait]
pub trait MessageRepository: Send + Sync {
    async fn insert(&self, msg: &NewMessage) -> Result<Message, DbError>;
    async fn find_by_id(&self, id: Uuid, account_id: Uuid) -> Result<Option<Message>, DbError>;
    async fn list_by_account(&self, account_id: Uuid, pagination: &Pagination) -> Result<Vec<Message>, DbError>;
    async fn update_status(&self, id: Uuid, status: &str, provider: Option<&str>, provider_message_id: Option<&str>, error_message: Option<&str>) -> Result<(), DbError>;
    async fn insert_delivery_event(&self, message_id: Uuid, status: &str, provider_data: Option<serde_json::Value>) -> Result<(), DbError>;
    async fn get_delivery_events(&self, message_id: Uuid) -> Result<Vec<DeliveryEvent>, DbError>;
}

#[async_trait]
pub trait ApiKeyRepository: Send + Sync {
    async fn list_by_account(&self, account_id: Uuid) -> Result<Vec<ApiKey>, DbError>;
    async fn insert(&self, account_id: Uuid, name: &str, key_hash: &str, key_prefix: &str, environment: &str) -> Result<ApiKey, DbError>;
    async fn revoke(&self, id: Uuid, account_id: Uuid) -> Result<(), DbError>;
}
```

### Step 3: Create sqlx implementation (db/postgres.rs)

Implement all three traits using sqlx `query_as!` macros against PgPool. Each method is a straightforward SQL query.

### Step 4: Verify compilation

Run: `cargo check --workspace`

### Step 5: Commit

```bash
git add crates/chorus-server/src/db/
git commit -m "feat(server): add database migrations and repository traits"
```

---

## Task 3: API Key Auth Middleware

### Files
- Create: `crates/chorus-server/src/auth/mod.rs`
- Create: `crates/chorus-server/src/auth/api_key.rs`

### Step 1: Create auth module

`auth/api_key.rs` — Axum extractor that:
1. Reads `Authorization: Bearer ch_live_xxx` header
2. SHA-256 hashes the key
3. Looks up hash in DB via `AccountRepository`
4. Returns `AccountContext { account_id, key_id, environment }` or 401

```rust
use axum::{
    extract::{FromRequestParts, State},
    http::{request::Parts, StatusCode},
};
use sha2::{Sha256, Digest};
use uuid::Uuid;
use std::sync::Arc;
use crate::app::AppState;

#[derive(Debug, Clone)]
pub struct AccountContext {
    pub account_id: Uuid,
    pub key_id: Uuid,
    pub environment: String,
}

#[axum::async_trait]
impl FromRequestParts<Arc<AppState>> for AccountContext {
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

        if !key.starts_with("ch_live_") && !key.starts_with("ch_test_") {
            return Err((StatusCode::UNAUTHORIZED, "invalid api key format"));
        }

        let hash = hex::encode(Sha256::digest(key.as_bytes()));

        let (account, api_key) = state
            .account_repo()
            .find_by_api_key_hash(&hash)
            .await
            .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "database error"))?
            .ok_or((StatusCode::UNAUTHORIZED, "invalid api key"))?;

        if !account.is_active || api_key.is_revoked {
            return Err((StatusCode::UNAUTHORIZED, "account or key is inactive"));
        }

        if let Some(expires_at) = api_key.expires_at {
            if expires_at < chrono::Utc::now() {
                return Err((StatusCode::UNAUTHORIZED, "api key expired"));
            }
        }

        // Update last_used_at in background
        let repo = state.account_repo();
        let key_id = api_key.id;
        tokio::spawn(async move { let _ = repo.update_key_last_used(key_id).await; });

        Ok(AccountContext {
            account_id: account.id,
            key_id: api_key.id,
            environment: api_key.environment,
        })
    }
}
```

### Step 2: Verify compilation

Run: `cargo check --workspace`

### Step 3: Commit

```bash
git add crates/chorus-server/src/auth/
git commit -m "feat(server): add API key auth middleware"
```

---

## Task 4: Health & Send Endpoints

### Files
- Create: `crates/chorus-server/src/routes/mod.rs`
- Create: `crates/chorus-server/src/routes/health.rs`
- Create: `crates/chorus-server/src/routes/sms.rs`
- Create: `crates/chorus-server/src/routes/email.rs`
- Modify: `crates/chorus-server/src/app.rs`

### Step 1: Create health endpoint

```rust
// routes/health.rs
use axum::Json;
use serde::Serialize;

#[derive(Serialize)]
pub struct HealthResponse {
    pub status: &'static str,
    pub version: &'static str,
}

pub async fn health() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        version: env!("CARGO_PKG_VERSION"),
    })
}
```

### Step 2: Create SMS send endpoint

```rust
// routes/sms.rs
use axum::{extract::State, http::StatusCode, Json};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use std::sync::Arc;
use crate::auth::api_key::AccountContext;
use crate::app::AppState;

#[derive(Deserialize)]
pub struct SendSmsRequest {
    pub to: String,
    pub body: String,
    pub from: Option<String>,
}

#[derive(Serialize)]
pub struct SendResponse {
    pub message_id: Uuid,
    pub status: &'static str,
}

pub async fn send_sms(
    State(state): State<Arc<AppState>>,
    ctx: AccountContext,
    Json(req): Json<SendSmsRequest>,
) -> Result<(StatusCode, Json<SendResponse>), (StatusCode, String)> {
    // Insert message to DB as queued
    // Enqueue job to Redis
    // Return 202
}
```

### Step 3: Create email send endpoint (same pattern as SMS)

### Step 4: Wire routes into app.rs router

```rust
pub fn create_router(state: AppState) -> Router {
    let state = Arc::new(state);
    Router::new()
        .route("/health", get(routes::health::health))
        .route("/v1/sms/send", post(routes::sms::send_sms))
        .route("/v1/email/send", post(routes::email::send_email))
        .with_state(state)
}
```

### Step 5: Verify compilation

Run: `cargo check --workspace`

### Step 6: Commit

```bash
git add crates/chorus-server/src/routes/ crates/chorus-server/src/app.rs
git commit -m "feat(server): add health, SMS send, and email send endpoints"
```

---

## Task 5: Redis Queue & Worker

### Files
- Create: `crates/chorus-server/src/queue/mod.rs`
- Create: `crates/chorus-server/src/queue/redis.rs`
- Create: `crates/chorus-server/src/queue/worker.rs`

### Step 1: Create job queue trait and Redis implementation

Queue interface:
- `enqueue(job: SendJob)` — push to Redis list
- `dequeue()` — blocking pop from Redis list
- Worker loop: dequeue → load account providers → send via chorus-core → update DB status

### Step 2: Create worker

Worker runs as background tokio task in main.rs:
- Dequeue jobs from Redis
- Build WaterfallRouter from account's provider config
- Call `router.send_sms()` or `router.send_email()`
- Update message status + insert delivery_event
- On failure: retry up to 3 times with exponential backoff (1s, 2s, 4s)

### Step 3: Wire worker into main.rs

Spawn worker as `tokio::spawn` in main.

### Step 4: Verify compilation

Run: `cargo check --workspace`

### Step 5: Commit

```bash
git add crates/chorus-server/src/queue/
git commit -m "feat(server): add Redis job queue and worker"
```

---

## Task 6: OTP Endpoints

### Files
- Create: `crates/chorus-server/src/otp/mod.rs`
- Create: `crates/chorus-server/src/routes/otp.rs`

### Step 1: Create OTP module

```rust
// otp/mod.rs
// - generate_code() -> 6-digit string
// - store(redis, recipient_hash, code) -> otp_id, TTL 5 min
// - verify(redis, otp_id, code) -> Result<bool>
//   - max 3 attempts, then lockout
//   - delete on success
```

### Step 2: Create OTP routes

```
POST /v1/otp/send   { to, app_name? }  -> { otp_id }
POST /v1/otp/verify  { otp_id, code }   -> { verified: bool }
```

### Step 3: Wire into router

### Step 4: Verify compilation

Run: `cargo check --workspace`

### Step 5: Commit

```bash
git add crates/chorus-server/src/otp/ crates/chorus-server/src/routes/otp.rs
git commit -m "feat(server): add OTP send and verify endpoints"
```

---

## Task 7: Messages & API Keys Endpoints

### Files
- Create: `crates/chorus-server/src/routes/messages.rs`
- Create: `crates/chorus-server/src/routes/keys.rs`

### Step 1: Messages endpoints

```
GET /v1/messages          -> paginated list (limit, offset query params)
GET /v1/messages/{id}     -> message detail + delivery_events
```

### Step 2: API keys endpoints

```
GET    /v1/keys           -> list keys (redacted, show prefix only)
POST   /v1/keys           -> create new key, return full key ONCE
DELETE /v1/keys/{id}      -> revoke (soft delete)
```

### Step 3: Wire into router

### Step 4: Verify compilation

Run: `cargo check --workspace`

### Step 5: Commit

```bash
git add crates/chorus-server/src/routes/messages.rs crates/chorus-server/src/routes/keys.rs
git commit -m "feat(server): add messages list/detail and API keys CRUD endpoints"
```

---

## Task 8: Docker Compose & Dockerfile

### Files
- Create: `docker-compose.yml`
- Create: `Dockerfile`
- Create: `.env.example`

### Step 1: Create Dockerfile (multi-stage)

```dockerfile
FROM rust:1.85 AS builder
WORKDIR /app
COPY . .
RUN cargo build --release -p chorus-server

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=builder /app/target/release/chorus-server /usr/local/bin/
EXPOSE 3000
CMD ["chorus-server"]
```

### Step 2: Create docker-compose.yml

PostgreSQL 16 + Redis 7 + chorus-server with healthchecks.

### Step 3: Create .env.example

```
DATABASE_URL=postgres://chorus:chorus@localhost:5432/chorus
REDIS_URL=redis://127.0.0.1:6379
HOST=0.0.0.0
PORT=3000
```

### Step 4: Verify docker compose builds

Run: `docker compose build`

### Step 5: Commit

```bash
git add docker-compose.yml Dockerfile .env.example
git commit -m "feat(server): add Dockerfile and Docker Compose for local dev"
```

---

## Task 9: Integration Tests & Final Verification

### Files
- Create: `crates/chorus-server/tests/api_test.rs`

### Step 1: Write integration tests

Use `axum::test` helpers to test:
- `GET /health` returns 200
- `POST /v1/sms/send` without auth returns 401
- `POST /v1/sms/send` with valid key returns 202
- `GET /v1/messages` returns paginated list
- `POST /v1/otp/send` + `POST /v1/otp/verify` flow

### Step 2: Run full check suite

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo doc --workspace --no-deps
```

### Step 3: Commit

```bash
git add crates/chorus-server/tests/
git commit -m "test(server): add integration tests for API endpoints"
```

---

## Verification Checklist

After all tasks complete:

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
docker compose build
docker compose up -d
curl http://localhost:3000/health
docker compose down
```

All must pass before merging.
