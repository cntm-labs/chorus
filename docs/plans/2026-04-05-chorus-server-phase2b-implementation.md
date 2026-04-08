# chorus-server Phase 2b Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Replace the placeholder worker with production-ready async queue processing — configurable worker pool, live provider delivery via chorus-core, delayed retry queue, and dead letter queue.

**Architecture:** Workers BRPOP from Redis, build a WaterfallRouter per job (per-account config or env defaults), send via chorus-core, retry failures via Redis sorted set with exponential backoff, push to DLQ after max retries.

**Tech Stack:** Rust, Axum 0.8, Redis 0.27 (BRPOP, ZADD, ZRANGEBYSCORE), chorus-core (WaterfallRouter), chorus-providers (Telnyx, Twilio, Plivo, Resend, SES, SMTP, Mock)

**Execution order:** Migration → ProviderConfig repo → Config env vars → Router builder → Delayed queue → Dead letter queue → Worker rewrite → Provider routes → Wire into app/main → Tests → Verify

---

## Task 1: Migration + ProviderConfig Type & Repository

### Files
- Create: `crates/chorus-server/src/db/migrations/002_provider_configs.sql`
- Modify: `crates/chorus-server/src/db/mod.rs`
- Create: `crates/chorus-server/src/db/provider_config.rs`
- Modify: `crates/chorus-server/src/db/postgres.rs`

### Step 1: Create migration

File: `crates/chorus-server/src/db/migrations/002_provider_configs.sql`

```sql
CREATE TABLE provider_configs (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    account_id UUID NOT NULL REFERENCES accounts(id),
    channel TEXT NOT NULL CHECK (channel IN ('sms', 'email')),
    provider TEXT NOT NULL,
    priority INTEGER NOT NULL,
    credentials JSONB NOT NULL,
    is_active BOOLEAN NOT NULL DEFAULT true,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (account_id, channel, provider)
);

CREATE INDEX idx_provider_configs_account_channel
    ON provider_configs(account_id, channel);
```

### Step 2: Add type and trait to db/mod.rs

Add after existing types:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct ProviderConfig {
    pub id: Uuid,
    pub account_id: Uuid,
    pub channel: String,
    pub provider: String,
    pub priority: i32,
    pub credentials: serde_json::Value,
    pub is_active: bool,
    pub created_at: DateTime<Utc>,
}

pub struct NewProviderConfig {
    pub account_id: Uuid,
    pub channel: String,
    pub provider: String,
    pub priority: i32,
    pub credentials: serde_json::Value,
}

#[async_trait]
pub trait ProviderConfigRepository: Send + Sync {
    /// List active provider configs for an account+channel, ordered by priority.
    async fn list_by_account_channel(
        &self,
        account_id: Uuid,
        channel: &str,
    ) -> Result<Vec<ProviderConfig>, DbError>;

    /// Insert a new provider config.
    async fn insert(&self, config: &NewProviderConfig) -> Result<ProviderConfig, DbError>;

    /// List all provider configs for an account.
    async fn list_by_account(&self, account_id: Uuid) -> Result<Vec<ProviderConfig>, DbError>;

    /// Delete a provider config.
    async fn delete(&self, id: Uuid, account_id: Uuid) -> Result<(), DbError>;
}
```

Add `pub mod provider_config;` to the module.

### Step 3: Implement in db/provider_config.rs

```rust
use async_trait::async_trait;
use sqlx::PgPool;
use uuid::Uuid;

use super::{DbError, NewProviderConfig, ProviderConfig, ProviderConfigRepository};

pub struct PgProviderConfigRepository {
    pool: PgPool,
}

impl PgProviderConfigRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl ProviderConfigRepository for PgProviderConfigRepository {
    async fn list_by_account_channel(
        &self,
        account_id: Uuid,
        channel: &str,
    ) -> Result<Vec<ProviderConfig>, DbError> {
        sqlx::query_as::<_, ProviderConfig>(
            "SELECT * FROM provider_configs
             WHERE account_id = $1 AND channel = $2 AND is_active = true
             ORDER BY priority ASC",
        )
        .bind(account_id)
        .bind(channel)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| DbError::Internal(e.into()))
    }

    async fn insert(&self, config: &NewProviderConfig) -> Result<ProviderConfig, DbError> {
        sqlx::query_as::<_, ProviderConfig>(
            "INSERT INTO provider_configs (account_id, channel, provider, priority, credentials)
             VALUES ($1, $2, $3, $4, $5)
             RETURNING *",
        )
        .bind(config.account_id)
        .bind(&config.channel)
        .bind(&config.provider)
        .bind(config.priority)
        .bind(&config.credentials)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| DbError::Internal(e.into()))
    }

    async fn list_by_account(&self, account_id: Uuid) -> Result<Vec<ProviderConfig>, DbError> {
        sqlx::query_as::<_, ProviderConfig>(
            "SELECT * FROM provider_configs WHERE account_id = $1 ORDER BY channel, priority ASC",
        )
        .bind(account_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| DbError::Internal(e.into()))
    }

    async fn delete(&self, id: Uuid, account_id: Uuid) -> Result<(), DbError> {
        let result = sqlx::query(
            "DELETE FROM provider_configs WHERE id = $1 AND account_id = $2",
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
```

### Step 4: Verify compilation

Run: `cargo check --workspace`

### Step 5: Commit

```bash
git add crates/chorus-server/src/db/
git commit -m "feat(server): add provider_configs migration and repository"
```

---

## Task 2: Config + Global Provider Env Vars

### Files
- Modify: `crates/chorus-server/src/config.rs`

### Step 1: Add provider and worker fields

```rust
/// Server configuration loaded from environment variables.
pub struct Config {
    pub database_url: String,
    pub redis_url: String,
    pub host: String,
    pub port: u16,
    pub worker_concurrency: usize,

    // SMS providers (global defaults)
    pub telnyx_api_key: Option<String>,
    pub telnyx_from: Option<String>,
    pub twilio_account_sid: Option<String>,
    pub twilio_auth_token: Option<String>,
    pub twilio_from: Option<String>,
    pub plivo_auth_id: Option<String>,
    pub plivo_auth_token: Option<String>,
    pub plivo_from: Option<String>,

    // Email providers (global defaults)
    pub resend_api_key: Option<String>,
    pub ses_access_key: Option<String>,
    pub ses_secret_key: Option<String>,
    pub ses_region: Option<String>,
    pub smtp_host: Option<String>,
    pub smtp_port: Option<u16>,
    pub smtp_username: Option<String>,
    pub smtp_password: Option<String>,
    pub from_email: Option<String>,
}

impl Config {
    /// Load configuration from environment variables with sensible defaults.
    pub fn from_env() -> Self {
        Self {
            database_url: std::env::var("DATABASE_URL")
                .unwrap_or_else(|_| "postgres://chorus:chorus@localhost:5432/chorus".into()),
            redis_url: std::env::var("REDIS_URL")
                .unwrap_or_else(|_| "redis://127.0.0.1:6379".into()),
            host: std::env::var("HOST").unwrap_or_else(|_| "0.0.0.0".into()),
            port: std::env::var("PORT")
                .ok()
                .and_then(|p| p.parse().ok())
                .unwrap_or(3000),
            worker_concurrency: std::env::var("WORKER_CONCURRENCY")
                .ok()
                .and_then(|p| p.parse().ok())
                .unwrap_or(4),

            telnyx_api_key: std::env::var("TELNYX_API_KEY").ok(),
            telnyx_from: std::env::var("TELNYX_FROM").ok(),
            twilio_account_sid: std::env::var("TWILIO_ACCOUNT_SID").ok(),
            twilio_auth_token: std::env::var("TWILIO_AUTH_TOKEN").ok(),
            twilio_from: std::env::var("TWILIO_FROM").ok(),
            plivo_auth_id: std::env::var("PLIVO_AUTH_ID").ok(),
            plivo_auth_token: std::env::var("PLIVO_AUTH_TOKEN").ok(),
            plivo_from: std::env::var("PLIVO_FROM").ok(),

            resend_api_key: std::env::var("RESEND_API_KEY").ok(),
            ses_access_key: std::env::var("AWS_SES_ACCESS_KEY").ok(),
            ses_secret_key: std::env::var("AWS_SES_SECRET_KEY").ok(),
            ses_region: std::env::var("AWS_SES_REGION").ok(),
            smtp_host: std::env::var("SMTP_HOST").ok(),
            smtp_port: std::env::var("SMTP_PORT").ok().and_then(|p| p.parse().ok()),
            smtp_username: std::env::var("SMTP_USERNAME").ok(),
            smtp_password: std::env::var("SMTP_PASSWORD").ok(),
            from_email: std::env::var("FROM_EMAIL").ok(),
        }
    }
}
```

### Step 2: Verify compilation

Run: `cargo check --workspace`

### Step 3: Commit

```bash
git add crates/chorus-server/src/config.rs
git commit -m "feat(server): add provider env vars and worker_concurrency to Config"
```

---

## Task 3: Router Builder

### Files
- Create: `crates/chorus-server/src/queue/router_builder.rs`
- Modify: `crates/chorus-server/src/queue/mod.rs`

### Step 1: Create router_builder.rs

Builds a `WaterfallRouter` from per-account DB config or global env defaults. Test mode always uses mock providers.

```rust
use chorus::router::WaterfallRouter;
use chorus_providers::email::mock::MockEmailSender;
use chorus_providers::email::resend::ResendEmailSender;
use chorus_providers::email::ses::SesEmailSender;
use chorus_providers::email::smtp::SmtpEmailSender;
use chorus_providers::sms::mock::MockSmsSender;
use chorus_providers::sms::plivo::PlivoSmsSender;
use chorus_providers::sms::telnyx::TelnyxSmsSender;
use chorus_providers::sms::twilio::TwilioSmsSender;
use std::sync::Arc;

use crate::config::Config;
use crate::db::ProviderConfig;

/// Build a router for test mode — always mock providers.
pub fn build_test_router(channel: &str) -> WaterfallRouter {
    let mut router = WaterfallRouter::new();
    match channel {
        "sms" => router = router.add_sms(Arc::new(MockSmsSender)),
        "email" => router = router.add_email(Arc::new(MockEmailSender)),
        _ => {}
    }
    router
}

/// Build a router from per-account provider configs (priority order).
pub fn build_router_from_configs(
    configs: &[ProviderConfig],
) -> anyhow::Result<WaterfallRouter> {
    let mut router = WaterfallRouter::new();

    for config in configs {
        router = add_provider_to_router(router, &config.provider, &config.channel, &config.credentials)?;
    }

    Ok(router)
}

/// Build a router from global env var defaults.
pub fn build_router_from_env(config: &Config, channel: &str) -> anyhow::Result<WaterfallRouter> {
    let mut router = WaterfallRouter::new();

    match channel {
        "sms" => {
            if let Some(ref api_key) = config.telnyx_api_key {
                router = router.add_sms(Arc::new(
                    TelnyxSmsSender::new(api_key.clone(), config.telnyx_from.clone()),
                ));
            }
            if let (Some(ref sid), Some(ref token)) =
                (&config.twilio_account_sid, &config.twilio_auth_token)
            {
                router = router.add_sms(Arc::new(
                    TwilioSmsSender::new(sid.clone(), token.clone(), config.twilio_from.clone()),
                ));
            }
            if let (Some(ref id), Some(ref token)) =
                (&config.plivo_auth_id, &config.plivo_auth_token)
            {
                router = router.add_sms(Arc::new(
                    PlivoSmsSender::new(id.clone(), token.clone(), config.plivo_from.clone()),
                ));
            }
        }
        "email" => {
            if let (Some(ref api_key), Some(ref from)) =
                (&config.resend_api_key, &config.from_email)
            {
                router = router.add_email(Arc::new(
                    ResendEmailSender::new(api_key.clone(), from.clone()),
                ));
            }
            if let (Some(ref ak), Some(ref sk), Some(ref region), Some(ref from)) = (
                &config.ses_access_key,
                &config.ses_secret_key,
                &config.ses_region,
                &config.from_email,
            ) {
                let sender = SesEmailSender::new(
                    ak.clone(),
                    sk.clone(),
                    region.clone(),
                    from.clone(),
                )?;
                router = router.add_email(Arc::new(sender));
            }
            if let (Some(ref host), Some(port), Some(ref user), Some(ref pass), Some(ref from)) = (
                &config.smtp_host,
                config.smtp_port,
                &config.smtp_username,
                &config.smtp_password,
                &config.from_email,
            ) {
                let sender = SmtpEmailSender::new(
                    host.clone(),
                    port,
                    user.clone(),
                    pass.clone(),
                    from.clone(),
                )?;
                router = router.add_email(Arc::new(sender));
            }
        }
        _ => {}
    }

    Ok(router)
}

/// Add a single provider to a router based on provider name and credentials JSON.
fn add_provider_to_router(
    router: WaterfallRouter,
    provider: &str,
    channel: &str,
    creds: &serde_json::Value,
) -> anyhow::Result<WaterfallRouter> {
    match (channel, provider) {
        ("sms", "telnyx") => {
            let api_key = creds["api_key"].as_str().unwrap_or_default().to_string();
            let from = creds["from"].as_str().map(String::from);
            Ok(router.add_sms(Arc::new(TelnyxSmsSender::new(api_key, from))))
        }
        ("sms", "twilio") => {
            let sid = creds["account_sid"].as_str().unwrap_or_default().to_string();
            let token = creds["auth_token"].as_str().unwrap_or_default().to_string();
            let from = creds["from"].as_str().map(String::from);
            Ok(router.add_sms(Arc::new(TwilioSmsSender::new(sid, token, from))))
        }
        ("sms", "plivo") => {
            let id = creds["auth_id"].as_str().unwrap_or_default().to_string();
            let token = creds["auth_token"].as_str().unwrap_or_default().to_string();
            let from = creds["from"].as_str().map(String::from);
            Ok(router.add_sms(Arc::new(PlivoSmsSender::new(id, token, from))))
        }
        ("email", "resend") => {
            let api_key = creds["api_key"].as_str().unwrap_or_default().to_string();
            let from = creds["from"].as_str().unwrap_or_default().to_string();
            Ok(router.add_email(Arc::new(ResendEmailSender::new(api_key, from))))
        }
        ("email", "ses") => {
            let ak = creds["access_key"].as_str().unwrap_or_default().to_string();
            let sk = creds["secret_key"].as_str().unwrap_or_default().to_string();
            let region = creds["region"].as_str().unwrap_or_default().to_string();
            let from = creds["from"].as_str().unwrap_or_default().to_string();
            let sender = SesEmailSender::new(ak, sk, region, from)?;
            Ok(router.add_email(Arc::new(sender)))
        }
        ("email", "smtp") => {
            let host = creds["host"].as_str().unwrap_or_default().to_string();
            let port = creds["port"].as_u64().unwrap_or(587) as u16;
            let user = creds["username"].as_str().unwrap_or_default().to_string();
            let pass = creds["password"].as_str().unwrap_or_default().to_string();
            let from = creds["from"].as_str().unwrap_or_default().to_string();
            let sender = SmtpEmailSender::new(host, port, user, pass, from)?;
            Ok(router.add_email(Arc::new(sender)))
        }
        _ => anyhow::bail!("unknown provider: {channel}/{provider}"),
    }
}
```

### Step 2: Register module in queue/mod.rs

Add `pub mod router_builder;`

### Step 3: Verify compilation

Run: `cargo check --workspace`

### Step 4: Commit

```bash
git add crates/chorus-server/src/queue/
git commit -m "feat(server): add router builder for per-account and global providers"
```

---

## Task 4: Delayed Queue + Dead Letter Queue

### Files
- Create: `crates/chorus-server/src/queue/delayed.rs`
- Create: `crates/chorus-server/src/queue/dead_letter.rs`
- Modify: `crates/chorus-server/src/queue/mod.rs`

### Step 1: Extract shared constants to queue/mod.rs

Replace existing content of `queue/mod.rs`:

```rust
pub mod dead_letter;
pub mod delayed;
pub mod enqueue;
pub mod router_builder;
pub mod worker;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Main work queue key.
pub const QUEUE_KEY: &str = "chorus:jobs";
/// Delayed retry queue (sorted set, score = retry-at Unix timestamp).
pub const DELAYED_KEY: &str = "chorus:delayed";
/// Dead letter queue for permanently failed jobs.
pub const DEAD_LETTER_KEY: &str = "chorus:dead_letters";
/// Maximum delivery attempts before moving to DLQ.
pub const MAX_RETRIES: i32 = 3;

/// A job representing a message to be sent.
#[derive(Debug, Serialize, Deserialize)]
pub struct SendJob {
    /// The message ID from the database.
    pub message_id: Uuid,
    /// The account that owns this message.
    pub account_id: Uuid,
    /// `"sms"` or `"email"`.
    pub channel: String,
    /// `"live"` or `"test"`.
    pub environment: String,
    /// Current attempt number (0-based, incremented on retry).
    pub attempt: i32,
}
```

### Step 2: Create delayed.rs

```rust
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

/// Spawn the delayed queue poller that moves due jobs back to the main queue.
pub fn spawn_delayed_poller(redis: redis::Client) {
    tokio::spawn(async move {
        tracing::info!("delayed queue poller started");
        loop {
            if let Err(e) = poll_delayed_jobs(&redis).await {
                tracing::error!("delayed poller error: {e}");
            }
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
    });
}

/// Move jobs whose retry-at timestamp has passed back to the main queue.
async fn poll_delayed_jobs(redis: &redis::Client) -> anyhow::Result<()> {
    let mut conn = redis.get_multiplexed_tokio_connection().await?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)?
        .as_secs();

    // Fetch jobs that are due (score <= now)
    let jobs: Vec<String> = redis::cmd("ZRANGEBYSCORE")
        .arg(super::DELAYED_KEY)
        .arg("-inf")
        .arg(now)
        .query_async(&mut conn)
        .await?;

    for payload in jobs {
        // Atomically remove from delayed set — if ZREM returns 0, another poller got it
        let removed: i64 = redis::cmd("ZREM")
            .arg(super::DELAYED_KEY)
            .arg(&payload)
            .query_async(&mut conn)
            .await?;

        if removed > 0 {
            redis::cmd("LPUSH")
                .arg(super::QUEUE_KEY)
                .arg(&payload)
                .query_async::<i64>(&mut conn)
                .await?;
            tracing::debug!("moved delayed job back to main queue");
        }
    }

    Ok(())
}

/// Schedule a job for retry after exponential backoff.
pub async fn schedule_retry(redis: &redis::Client, job: &super::SendJob) -> anyhow::Result<()> {
    let mut conn = redis.get_multiplexed_tokio_connection().await?;
    let backoff_secs = 1u64 << job.attempt.min(10) as u64; // 2^attempt: 1, 2, 4, ...
    let retry_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)?
        .as_secs()
        + backoff_secs;

    let payload = serde_json::to_string(job)?;
    redis::cmd("ZADD")
        .arg(super::DELAYED_KEY)
        .arg(retry_at)
        .arg(payload)
        .query_async::<i64>(&mut conn)
        .await?;

    tracing::debug!(
        message_id = %job.message_id,
        attempt = job.attempt,
        backoff_secs,
        "scheduled retry"
    );

    Ok(())
}
```

### Step 3: Create dead_letter.rs

```rust
/// Push a failed job to the dead letter queue.
pub async fn push_to_dlq(redis: &redis::Client, job: &super::SendJob) -> anyhow::Result<()> {
    let mut conn = redis.get_multiplexed_tokio_connection().await?;
    let payload = serde_json::to_string(job)?;

    redis::cmd("LPUSH")
        .arg(super::DEAD_LETTER_KEY)
        .arg(payload)
        .query_async::<i64>(&mut conn)
        .await?;

    tracing::warn!(message_id = %job.message_id, "job moved to dead letter queue");
    Ok(())
}
```

### Step 4: Update enqueue.rs to use shared constant

```rust
use crate::app::AppState;

/// Push a send job onto the Redis queue.
pub async fn enqueue_job(state: &AppState, job: &super::SendJob) -> anyhow::Result<()> {
    let mut conn = state.redis.get_multiplexed_tokio_connection().await?;
    let payload = serde_json::to_string(job)?;
    redis::cmd("LPUSH")
        .arg(super::QUEUE_KEY)
        .arg(payload)
        .query_async::<i64>(&mut conn)
        .await?;
    Ok(())
}
```

### Step 5: Verify compilation

Run: `cargo check --workspace`

### Step 6: Commit

```bash
git add crates/chorus-server/src/queue/
git commit -m "feat(server): add delayed retry queue and dead letter queue"
```

---

## Task 5: Rewrite Worker with Pool + Live Delivery

### Files
- Rewrite: `crates/chorus-server/src/queue/worker.rs`

### Step 1: Rewrite worker.rs

```rust
use chorus::types::{EmailMessage, SmsMessage};
use std::sync::Arc;

use crate::app::AppState;
use crate::config::Config;

/// Spawn N worker tasks that process jobs from the main queue.
pub fn spawn_workers(state: Arc<AppState>, config: Arc<Config>, concurrency: usize) {
    for i in 0..concurrency {
        let state = Arc::clone(&state);
        let config = Arc::clone(&config);
        tokio::spawn(async move {
            tracing::info!(worker = i, "queue worker started");
            loop {
                if let Err(e) = process_next_job(&state, &config).await {
                    tracing::error!(worker = i, "worker error: {e}");
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                }
            }
        });
    }
}

/// Block-pop the next job from Redis and process it.
async fn process_next_job(state: &Arc<AppState>, config: &Config) -> anyhow::Result<()> {
    let mut conn = state.redis.get_multiplexed_tokio_connection().await?;

    let result: Option<(String, String)> = redis::cmd("BRPOP")
        .arg(super::QUEUE_KEY)
        .arg(5)
        .query_async(&mut conn)
        .await?;

    let Some((_key, payload)) = result else {
        return Ok(());
    };

    let job: super::SendJob = serde_json::from_str(&payload)?;
    let repo = state.message_repo();

    // Load the message from DB
    let message = repo
        .find_by_id(job.message_id, job.account_id)
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?
        .ok_or_else(|| anyhow::anyhow!("message {} not found", job.message_id))?;

    // Max retries exceeded → DLQ
    if job.attempt >= super::MAX_RETRIES {
        repo.update_status(
            job.message_id,
            "failed",
            None,
            None,
            Some("max retries exceeded"),
        )
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;
        repo.insert_delivery_event(
            job.message_id,
            "failed",
            Some(serde_json::json!({"reason": "max retries exceeded", "attempts": job.attempt})),
        )
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;

        super::dead_letter::push_to_dlq(&state.redis, &job).await?;
        return Ok(());
    }

    // Build router based on environment
    let router = if job.environment == "test" {
        super::router_builder::build_test_router(&job.channel)
    } else {
        // Try per-account config, fall back to env defaults
        let configs = state
            .provider_config_repo()
            .list_by_account_channel(job.account_id, &job.channel)
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))?;

        if configs.is_empty() {
            super::router_builder::build_router_from_env(config, &job.channel)?
        } else {
            super::router_builder::build_router_from_configs(&configs)?
        }
    };

    // Send via chorus-core
    let send_result = match job.channel.as_str() {
        "sms" => {
            let msg = SmsMessage {
                to: message.recipient.clone(),
                body: message.body.clone(),
                from: message.sender.clone(),
            };
            router.send_sms(&msg).await
        }
        "email" => {
            let msg = EmailMessage {
                to: message.recipient.clone(),
                subject: message.subject.clone().unwrap_or_default(),
                html_body: message.body.clone(),
                text_body: message.body.clone(),
                from: message.sender.clone(),
            };
            router.send_email(&msg).await
        }
        _ => anyhow::bail!("unknown channel: {}", job.channel),
    };

    match send_result {
        Ok(result) => {
            repo.update_status(
                job.message_id,
                "delivered",
                Some(&result.provider),
                Some(&result.message_id),
                None,
            )
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))?;
            repo.insert_delivery_event(
                job.message_id,
                "delivered",
                Some(serde_json::json!({
                    "provider": result.provider,
                    "provider_message_id": result.message_id,
                })),
            )
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        }
        Err(e) => {
            let error_msg = e.to_string();
            repo.update_status(
                job.message_id,
                "retrying",
                None,
                None,
                Some(&error_msg),
            )
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))?;
            repo.insert_delivery_event(
                job.message_id,
                "failed_attempt",
                Some(serde_json::json!({
                    "attempt": job.attempt,
                    "error": error_msg,
                })),
            )
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))?;

            // Schedule retry with incremented attempt
            let retry_job = super::SendJob {
                attempt: job.attempt + 1,
                ..job
            };
            super::delayed::schedule_retry(&state.redis, &retry_job).await?;
        }
    }

    Ok(())
}
```

### Step 2: Verify compilation

Run: `cargo check --workspace`

### Step 3: Commit

```bash
git add crates/chorus-server/src/queue/worker.rs
git commit -m "feat(server): rewrite worker with pool, live delivery, retry, and DLQ"
```

---

## Task 6: Provider Config Routes

### Files
- Create: `crates/chorus-server/src/routes/provider_configs.rs`
- Modify: `crates/chorus-server/src/routes/mod.rs`

### Step 1: Create routes

```rust
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::Deserialize;
use std::sync::Arc;
use uuid::Uuid;

use crate::app::AppState;
use crate::auth::api_key::AccountContext;
use crate::db::{NewProviderConfig, ProviderConfig};

/// Request body for adding a provider config.
#[derive(Deserialize)]
pub struct CreateProviderConfigRequest {
    pub channel: String,
    pub provider: String,
    pub priority: i32,
    pub credentials: serde_json::Value,
}

/// List all provider configs for the authenticated account.
pub async fn list_provider_configs(
    State(state): State<Arc<AppState>>,
    ctx: AccountContext,
) -> Result<Json<Vec<ProviderConfig>>, (StatusCode, String)> {
    let configs = state
        .provider_config_repo()
        .list_by_account(ctx.account_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(configs))
}

/// Add a provider config for the authenticated account.
pub async fn create_provider_config(
    State(state): State<Arc<AppState>>,
    ctx: AccountContext,
    Json(req): Json<CreateProviderConfigRequest>,
) -> Result<(StatusCode, Json<ProviderConfig>), (StatusCode, String)> {
    let valid_channels = ["sms", "email"];
    if !valid_channels.contains(&req.channel.as_str()) {
        return Err((StatusCode::BAD_REQUEST, "channel must be 'sms' or 'email'".into()));
    }

    let valid_providers = ["telnyx", "twilio", "plivo", "resend", "ses", "smtp"];
    if !valid_providers.contains(&req.provider.as_str()) {
        return Err((StatusCode::BAD_REQUEST, format!("unknown provider: {}", req.provider)));
    }

    let config = state
        .provider_config_repo()
        .insert(&NewProviderConfig {
            account_id: ctx.account_id,
            channel: req.channel,
            provider: req.provider,
            priority: req.priority,
            credentials: req.credentials,
        })
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok((StatusCode::CREATED, Json(config)))
}

/// Delete a provider config.
pub async fn delete_provider_config(
    State(state): State<Arc<AppState>>,
    ctx: AccountContext,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, (StatusCode, String)> {
    state
        .provider_config_repo()
        .delete(id, ctx.account_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(StatusCode::NO_CONTENT)
}
```

### Step 2: Register module

Add `pub mod provider_configs;` to `routes/mod.rs`.

### Step 3: Verify compilation

Run: `cargo check --workspace`

### Step 4: Commit

```bash
git add crates/chorus-server/src/routes/
git commit -m "feat(server): add provider config CRUD endpoints"
```

---

## Task 7: Wire Everything into AppState + Main

### Files
- Modify: `crates/chorus-server/src/app.rs`
- Modify: `crates/chorus-server/src/main.rs`
- Modify: `crates/chorus-server/src/routes/sms.rs`
- Modify: `crates/chorus-server/src/routes/email.rs`
- Modify: `crates/chorus-server/src/routes/otp.rs`

### Step 1: Update AppState

Add `ProviderConfigRepository` + `Config` to `AppState`:

- Add `provider_config_repo: Arc<dyn ProviderConfigRepository>` field
- Add `config: Arc<Config>` field
- Update `new()` to create `PgProviderConfigRepository`
- Add `provider_config_repo()` accessor
- Add `config()` accessor
- Update `with_repos()` for testing (accept provider_config_repo param)
- Add provider config routes to `create_router()`

### Step 2: Update main.rs

- Create `Arc<Config>` and pass to `AppState::new()`
- Replace `spawn_worker(state)` with `spawn_workers(state, config, config.worker_concurrency)`
- Add `delayed::spawn_delayed_poller(redis.clone())`

### Step 3: Update send routes (sms.rs, email.rs, otp.rs)

Add `attempt: 0` to `SendJob` construction in all three files.

### Step 4: Verify compilation

Run: `cargo check --workspace`

### Step 5: Commit

```bash
git add crates/chorus-server/src/
git commit -m "feat(server): wire provider config repo, worker pool, and delayed poller into app"
```

---

## Task 8: Update Tests

### Files
- Modify: `crates/chorus-server/tests/api_test.rs`

### Step 1: Update mock repos and test_state

- Add `MockProviderConfigRepo` implementing `ProviderConfigRepository`
- Update `test_state()` to pass the new mock + dummy Config
- Update `AppState::with_repos()` call signature
- Add test for `GET /v1/providers` returns 200

### Step 2: Run full test suite

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

### Step 3: Commit

```bash
git add crates/chorus-server/tests/
git commit -m "test(server): update integration tests for Phase 2b worker and provider config"
```

---

## Task 9: Update .env.example + Docker Compose

### Files
- Modify: `.env.example`
- Modify: `docker-compose.yml`

### Step 1: Add new env vars to .env.example

Add `WORKER_CONCURRENCY=4` and `TELNYX_FROM`, `TWILIO_FROM`, `PLIVO_FROM` placeholders.

### Step 2: Add WORKER_CONCURRENCY to docker-compose.yml

Add `WORKER_CONCURRENCY: "4"` to chorus-server environment.

### Step 3: Commit

```bash
git add .env.example docker-compose.yml
git commit -m "chore: update env example and docker-compose for Phase 2b"
```

---

## Verification Checklist

After all tasks complete:

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
cargo deny check
docker compose build
```

All must pass before merging.
