# Three Features Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add Mailgun email provider (#11), webhook callbacks (#16), and batch send endpoints (#17) to Chorus.

**Architecture:** Three independent features delivered as separate PRs. Mailgun follows the existing Resend provider pattern. Webhooks add a new DB table + routes + worker integration. Batch send reuses existing single-send infra with loop + Redis pipeline.

**Tech Stack:** Rust, Axum, reqwest, sqlx, Redis, wiremock (tests), HMAC-SHA256 (webhooks)

---

## PR 1: Mailgun Email Provider (#11)

### Task 1: Create Mailgun provider struct and constructor

**Files:**
- Create: `crates/chorus-providers/src/email/mailgun.rs`
- Modify: `crates/chorus-providers/src/email/mod.rs`

**Step 1: Write the failing test**

In `crates/chorus-providers/src/email/mailgun.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mailgun_provider_name() {
        let sender = MailgunEmailSender::new(
            "key-xxx".into(),
            "mg.example.com".into(),
            "noreply@example.com".into(),
        );
        assert_eq!(sender.provider_name(), "mailgun");
    }
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p chorus-providers mailgun_provider_name`
Expected: FAIL — module/struct not found

**Step 3: Write minimal implementation**

In `crates/chorus-providers/src/email/mailgun.rs`:

```rust
use async_trait::async_trait;
use chorus::email::EmailSender;
use chorus::error::ChorusError;
use chorus::types::{Channel, DeliveryStatus, EmailMessage, SendResult};
use chrono::Utc;
use serde::Deserialize;

/// Mailgun email provider.
///
/// Supports US and EU regions via configurable `base_url`.
pub struct MailgunEmailSender {
    api_key: String,
    domain: String,
    from: String,
    http_client: reqwest::Client,
    base_url: String,
}

impl MailgunEmailSender {
    /// Create a new Mailgun sender with US region default.
    pub fn new(api_key: String, domain: String, from: String) -> Self {
        Self {
            api_key,
            domain,
            from,
            http_client: reqwest::Client::new(),
            base_url: "https://api.mailgun.net".into(),
        }
    }

    /// Override the base URL (for EU region or testing).
    pub fn with_base_url(mut self, base_url: String) -> Self {
        self.base_url = base_url;
        self
    }
}

#[derive(Deserialize)]
struct MailgunResponse {
    id: String,
}

#[async_trait]
impl EmailSender for MailgunEmailSender {
    fn provider_name(&self) -> &str {
        "mailgun"
    }

    async fn send(&self, msg: &EmailMessage) -> Result<SendResult, ChorusError> {
        let from = msg.from.as_deref().unwrap_or(&self.from);
        let url = format!("{}/v3/{}/messages", self.base_url, self.domain);

        let form = reqwest::multipart::Form::new()
            .text("from", from.to_string())
            .text("to", msg.to.clone())
            .text("subject", msg.subject.clone())
            .text("html", msg.html_body.clone())
            .text("text", msg.text_body.clone());

        let resp = self
            .http_client
            .post(&url)
            .basic_auth("api", Some(&self.api_key))
            .multipart(form)
            .send()
            .await
            .map_err(|e| ChorusError::Provider {
                provider: "mailgun".into(),
                message: format!("HTTP error: {}", e),
            })?;

        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(ChorusError::Provider {
                provider: "mailgun".into(),
                message: format!("API error: {}", body),
            });
        }

        let mg_resp: MailgunResponse =
            resp.json().await.map_err(|e| ChorusError::Provider {
                provider: "mailgun".into(),
                message: format!("parse error: {}", e),
            })?;

        Ok(SendResult {
            message_id: mg_resp.id,
            provider: "mailgun".to_string(),
            channel: Channel::Email,
            status: DeliveryStatus::Sent,
            created_at: Utc::now(),
        })
    }
}
```

Add to `crates/chorus-providers/src/email/mod.rs`:

```rust
pub mod mailgun;
```

**Step 4: Run test to verify it passes**

Run: `cargo test -p chorus-providers mailgun_provider_name`
Expected: PASS

**Step 5: Commit**

```bash
git add crates/chorus-providers/src/email/mailgun.rs crates/chorus-providers/src/email/mod.rs
git commit -m "feat(providers): add Mailgun email provider struct and trait impl"
```

---

### Task 2: Add wiremock integration tests for Mailgun

**Files:**
- Modify: `crates/chorus-providers/src/email/mailgun.rs`

**Step 1: Write the failing tests**

Add to the `tests` module in `mailgun.rs`:

```rust
#[tokio::test]
async fn mailgun_send_success() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v3/mg.example.com/messages"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({"id": "<msg-123>", "message": "Queued"})),
        )
        .mount(&mock_server)
        .await;

    let sender = MailgunEmailSender::new(
        "key-xxx".into(),
        "mg.example.com".into(),
        "noreply@example.com".into(),
    )
    .with_base_url(mock_server.uri());

    let msg = EmailMessage {
        to: "user@test.com".into(),
        subject: "Test".into(),
        html_body: "<p>Hi</p>".into(),
        text_body: "Hi".into(),
        from: None,
    };

    let result = sender.send(&msg).await.unwrap();
    assert_eq!(result.provider, "mailgun");
    assert_eq!(result.message_id, "<msg-123>");
    assert!(matches!(result.status, DeliveryStatus::Sent));
}

#[tokio::test]
async fn mailgun_send_api_error() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v3/mg.example.com/messages"))
        .respond_with(ResponseTemplate::new(401).set_body_string("Forbidden"))
        .mount(&mock_server)
        .await;

    let sender = MailgunEmailSender::new(
        "bad-key".into(),
        "mg.example.com".into(),
        "noreply@example.com".into(),
    )
    .with_base_url(mock_server.uri());

    let msg = EmailMessage {
        to: "user@test.com".into(),
        subject: "Test".into(),
        html_body: "<p>Hi</p>".into(),
        text_body: "Hi".into(),
        from: None,
    };

    let err = sender.send(&msg).await.unwrap_err();
    assert!(matches!(err, ChorusError::Provider { .. }));
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test -p chorus-providers mailgun_send`
Expected: FAIL (tests don't exist yet if step 1 isn't applied)

**Step 3: Tests are already written above — apply them**

**Step 4: Run tests to verify they pass**

Run: `cargo test -p chorus-providers mailgun_send`
Expected: PASS (both tests)

**Step 5: Commit**

```bash
git add crates/chorus-providers/src/email/mailgun.rs
git commit -m "test(providers): add wiremock tests for Mailgun send success and error"
```

---

### Task 3: Integrate Mailgun into server router builder and config

**Files:**
- Modify: `crates/chorus-server/src/config.rs`
- Modify: `crates/chorus-server/src/queue/router_builder.rs`

**Step 1: Add Mailgun config fields**

In `crates/chorus-server/src/config.rs`, add fields after `from_email`:

```rust
    /// Mailgun API key.
    pub mailgun_api_key: Option<String>,
    /// Mailgun sending domain.
    pub mailgun_domain: Option<String>,
    /// Mailgun base URL (default US, set to https://api.eu.mailgun.net for EU).
    pub mailgun_base_url: Option<String>,
```

In `Config::from_env()`, add after `from_email` line:

```rust
            mailgun_api_key: std::env::var("MAILGUN_API_KEY").ok(),
            mailgun_domain: std::env::var("MAILGUN_DOMAIN").ok(),
            mailgun_base_url: std::env::var("MAILGUN_BASE_URL").ok(),
```

**Step 2: Add Mailgun to router_builder.rs**

In `router_builder.rs`, add import:

```rust
use chorus_providers::email::mailgun::MailgunEmailSender;
```

In `build_router_from_env`, add after the SMTP block (before the closing `_ => {}`):

```rust
            if let (Some(ref api_key), Some(ref domain), Some(ref from)) =
                (&config.mailgun_api_key, &config.mailgun_domain, &config.from_email)
            {
                let mut sender = MailgunEmailSender::new(
                    api_key.clone(),
                    domain.clone(),
                    from.clone(),
                );
                if let Some(ref base_url) = config.mailgun_base_url {
                    sender = sender.with_base_url(base_url.clone());
                }
                router = router.add_email(Arc::new(sender));
            }
```

In `add_provider_to_router`, add match arm before `_ =>`:

```rust
        ("email", "mailgun") => {
            let api_key = creds["api_key"].as_str().unwrap_or_default().to_string();
            let domain = creds["domain"].as_str().unwrap_or_default().to_string();
            let from = creds["from"].as_str().unwrap_or_default().to_string();
            let mut sender = MailgunEmailSender::new(api_key, domain, from);
            if let Some(base_url) = creds["base_url"].as_str() {
                sender = sender.with_base_url(base_url.to_string());
            }
            Ok(router.add_email(Arc::new(sender)))
        }
```

**Step 3: Run full check**

Run: `cargo check --workspace`
Expected: PASS

**Step 4: Run all tests**

Run: `cargo test --workspace`
Expected: PASS

**Step 5: Commit**

```bash
git add crates/chorus-server/src/config.rs crates/chorus-server/src/queue/router_builder.rs
git commit -m "feat(server): integrate Mailgun provider into router builder and config"
```

---

### Task 4: Lint, format, and create PR for Mailgun

**Step 1: Run clippy and fmt**

```bash
cargo fmt --all
cargo clippy --workspace -- -D warnings
```

**Step 2: Fix any issues**

**Step 3: Commit fixes if any**

```bash
git commit -am "style: fix clippy and fmt warnings"
```

**Step 4: Create PR**

```bash
git checkout -b feat/mailgun-provider
git push -u origin feat/mailgun-provider
gh pr create --title "feat(providers): add Mailgun email provider" \
  --body "Closes #11" --label "enhancement,providers" --assignee "MrBT-nano"
```

---

## PR 2: Webhook Callbacks (#16)

### Task 5: Add webhook DB types and repository trait

**Files:**
- Modify: `crates/chorus-server/src/db/mod.rs`

**Step 1: Add Webhook types and trait to db/mod.rs**

After the `ApiKeyRepository` trait, add:

```rust
/// A webhook registration for delivery callbacks.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Webhook {
    pub id: Uuid,
    pub account_id: Uuid,
    pub url: String,
    pub secret: String,
    pub events: Vec<String>,
    pub is_active: bool,
    pub created_at: DateTime<Utc>,
}

/// Parameters for registering a new webhook.
pub struct NewWebhook {
    pub account_id: Uuid,
    pub url: String,
    pub secret: String,
    pub events: Vec<String>,
}

/// Webhook registration management.
#[async_trait]
pub trait WebhookRepository: Send + Sync {
    /// Insert a new webhook.
    async fn insert(&self, webhook: &NewWebhook) -> Result<Webhook, DbError>;

    /// List all active webhooks for an account.
    async fn list_by_account(&self, account_id: Uuid) -> Result<Vec<Webhook>, DbError>;

    /// List webhooks matching an account and event type.
    async fn list_by_account_event(
        &self,
        account_id: Uuid,
        event: &str,
    ) -> Result<Vec<Webhook>, DbError>;

    /// Delete a webhook.
    async fn delete(&self, id: Uuid, account_id: Uuid) -> Result<(), DbError>;
}
```

**Step 2: Run check**

Run: `cargo check --workspace`
Expected: PASS (trait is defined but not yet implemented)

**Step 3: Commit**

```bash
git add crates/chorus-server/src/db/mod.rs
git commit -m "feat(server): add Webhook DB types and repository trait"
```

---

### Task 6: Implement PostgreSQL webhook repository

**Files:**
- Create: `crates/chorus-server/src/db/webhook.rs`
- Modify: `crates/chorus-server/src/db/mod.rs` (add `pub mod webhook;`)

**Step 1: Create SQL migration**

Create `crates/chorus-server/migrations/XXXXXX_create_webhooks.sql`:

```sql
CREATE TABLE IF NOT EXISTS webhooks (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    account_id  UUID NOT NULL REFERENCES accounts(id),
    url         TEXT NOT NULL,
    secret      TEXT NOT NULL,
    events      TEXT[] NOT NULL,
    is_active   BOOLEAN NOT NULL DEFAULT true,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_webhooks_account_id ON webhooks(account_id);
```

**Step 2: Implement PgWebhookRepository**

In `crates/chorus-server/src/db/webhook.rs`:

```rust
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
```

**Step 3: Run check**

Run: `cargo check --workspace`
Expected: PASS

**Step 4: Commit**

```bash
git add crates/chorus-server/src/db/webhook.rs crates/chorus-server/src/db/mod.rs crates/chorus-server/migrations/
git commit -m "feat(server): implement PostgreSQL webhook repository"
```

---

### Task 7: Add webhook routes (POST, GET, DELETE)

**Files:**
- Create: `crates/chorus-server/src/routes/webhooks.rs`
- Modify: `crates/chorus-server/src/routes/mod.rs`
- Modify: `crates/chorus-server/src/app.rs`

**Step 1: Create webhook route handlers**

In `crates/chorus-server/src/routes/webhooks.rs`:

```rust
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use rand::Rng;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use crate::app::AppState;
use crate::auth::api_key::AccountContext;
use crate::db::NewWebhook;

/// Request body for registering a webhook.
#[derive(Deserialize)]
pub struct CreateWebhookRequest {
    /// Callback URL to receive events.
    pub url: String,
    /// Event types to subscribe to.
    pub events: Vec<String>,
}

/// Response after creating a webhook.
#[derive(Serialize)]
pub struct WebhookResponse {
    pub id: Uuid,
    pub url: String,
    pub secret: String,
    pub events: Vec<String>,
    pub created_at: String,
}

/// Response for listing webhooks (secret redacted).
#[derive(Serialize)]
pub struct WebhookListItem {
    pub id: Uuid,
    pub url: String,
    pub events: Vec<String>,
    pub created_at: String,
}

const VALID_EVENTS: &[&str] = &[
    "message.queued",
    "message.sent",
    "message.delivered",
    "message.failed",
];

/// Register a new webhook.
pub async fn create_webhook(
    State(state): State<Arc<AppState>>,
    ctx: AccountContext,
    Json(req): Json<CreateWebhookRequest>,
) -> Result<(StatusCode, Json<WebhookResponse>), (StatusCode, String)> {
    // Validate events
    for event in &req.events {
        if !VALID_EVENTS.contains(&event.as_str()) {
            return Err((
                StatusCode::BAD_REQUEST,
                format!("invalid event type: {event}"),
            ));
        }
    }

    // Generate HMAC signing secret
    let secret: String = hex::encode(rand::rng().random::<[u8; 32]>());

    let webhook = NewWebhook {
        account_id: ctx.account_id,
        url: req.url,
        secret: secret.clone(),
        events: req.events,
    };

    let created = state
        .webhook_repo()
        .insert(&webhook)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok((
        StatusCode::CREATED,
        Json(WebhookResponse {
            id: created.id,
            url: created.url,
            secret,
            events: created.events,
            created_at: created.created_at.to_rfc3339(),
        }),
    ))
}

/// List all active webhooks for the account.
pub async fn list_webhooks(
    State(state): State<Arc<AppState>>,
    ctx: AccountContext,
) -> Result<Json<Vec<WebhookListItem>>, (StatusCode, String)> {
    let webhooks = state
        .webhook_repo()
        .list_by_account(ctx.account_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let items: Vec<WebhookListItem> = webhooks
        .into_iter()
        .map(|w| WebhookListItem {
            id: w.id,
            url: w.url,
            events: w.events,
            created_at: w.created_at.to_rfc3339(),
        })
        .collect();

    Ok(Json(items))
}

/// Delete a webhook.
pub async fn delete_webhook(
    State(state): State<Arc<AppState>>,
    ctx: AccountContext,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, (StatusCode, String)> {
    state
        .webhook_repo()
        .delete(id, ctx.account_id)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;

    Ok(StatusCode::NO_CONTENT)
}
```

**Step 2: Wire into AppState and router**

Add `webhook_repo` field to `AppState` in `app.rs` (same pattern as other repos).

Add `pub mod webhooks;` to `routes/mod.rs`.

Add routes to `create_router` in `app.rs`:

```rust
        .route(
            "/v1/webhooks",
            get(routes::webhooks::list_webhooks).post(routes::webhooks::create_webhook),
        )
        .route(
            "/v1/webhooks/{id}",
            delete(routes::webhooks::delete_webhook),
        )
```

**Step 3: Run check**

Run: `cargo check --workspace`
Expected: PASS

**Step 4: Commit**

```bash
git add crates/chorus-server/src/routes/webhooks.rs crates/chorus-server/src/routes/mod.rs crates/chorus-server/src/app.rs
git commit -m "feat(server): add webhook CRUD routes (POST, GET, DELETE /v1/webhooks)"
```

---

### Task 8: Add webhook dispatcher to worker

**Files:**
- Create: `crates/chorus-server/src/queue/webhook_dispatch.rs`
- Modify: `crates/chorus-server/src/queue/mod.rs`
- Modify: `crates/chorus-server/src/queue/worker.rs`

**Step 1: Create webhook dispatcher**

In `crates/chorus-server/src/queue/webhook_dispatch.rs`:

```rust
use chrono::Utc;
use hmac::{Hmac, Mac};
use serde::Serialize;
use sha2::Sha256;
use std::sync::Arc;
use uuid::Uuid;

use crate::app::AppState;

type HmacSha256 = Hmac<Sha256>;

/// Webhook event payload sent to callback URLs.
#[derive(Serialize)]
pub struct WebhookPayload {
    pub event: String,
    pub message_id: Uuid,
    pub channel: String,
    pub provider: Option<String>,
    pub status: String,
    pub timestamp: String,
}

/// Dispatch webhook events for a message status change.
pub async fn dispatch_webhooks(
    state: &Arc<AppState>,
    account_id: Uuid,
    event: &str,
    payload: &WebhookPayload,
) {
    let webhooks = match state
        .webhook_repo()
        .list_by_account_event(account_id, event)
        .await
    {
        Ok(hooks) => hooks,
        Err(e) => {
            tracing::error!("failed to load webhooks: {e}");
            return;
        }
    };

    let body = match serde_json::to_string(payload) {
        Ok(b) => b,
        Err(e) => {
            tracing::error!("failed to serialize webhook payload: {e}");
            return;
        }
    };

    let client = reqwest::Client::new();
    let timestamp = Utc::now().timestamp().to_string();

    for webhook in webhooks {
        let signature = compute_signature(&webhook.secret, &body);

        let result = client
            .post(&webhook.url)
            .header("Content-Type", "application/json")
            .header("X-Chorus-Signature", &signature)
            .header("X-Chorus-Event", event)
            .header("X-Chorus-Timestamp", &timestamp)
            .body(body.clone())
            .send()
            .await;

        match result {
            Ok(resp) if resp.status().is_success() => {
                tracing::debug!(webhook_id = %webhook.id, "webhook delivered");
            }
            Ok(resp) => {
                tracing::warn!(
                    webhook_id = %webhook.id,
                    status = %resp.status(),
                    "webhook delivery failed"
                );
            }
            Err(e) => {
                tracing::warn!(webhook_id = %webhook.id, "webhook HTTP error: {e}");
            }
        }
    }
}

/// Compute HMAC-SHA256 signature for webhook payload.
fn compute_signature(secret: &str, body: &str) -> String {
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes())
        .expect("HMAC accepts any key length");
    mac.update(body.as_bytes());
    hex::encode(mac.finalize().into_bytes())
}
```

**Step 2: Add `hmac` and `sha2` to chorus-server Cargo.toml**

`sha2` is already a dependency. Add `hmac`:

```toml
hmac = { version = "0.12", features = ["std"] }
```

**Step 3: Integrate into worker.rs**

In `worker.rs`, after the `Ok(result)` block where status is updated to "delivered", add:

```rust
            // Dispatch webhook
            let payload = super::webhook_dispatch::WebhookPayload {
                event: "message.delivered".into(),
                message_id: job.message_id,
                channel: job.channel.clone(),
                provider: Some(result.provider.clone()),
                status: "delivered".into(),
                timestamp: Utc::now().to_rfc3339(),
            };
            super::webhook_dispatch::dispatch_webhooks(
                state, job.account_id, "message.delivered", &payload,
            )
            .await;
```

Similarly in the `Err(e)` block when max retries hit (status "failed"), add:

```rust
            let payload = super::webhook_dispatch::WebhookPayload {
                event: "message.failed".into(),
                message_id: job.message_id,
                channel: job.channel.clone(),
                provider: None,
                status: "failed".into(),
                timestamp: Utc::now().to_rfc3339(),
            };
            super::webhook_dispatch::dispatch_webhooks(
                state, job.account_id, "message.failed", &payload,
            )
            .await;
```

Add `pub mod webhook_dispatch;` to `queue/mod.rs`.

**Step 4: Run check and tests**

Run: `cargo check --workspace && cargo test --workspace`
Expected: PASS

**Step 5: Commit**

```bash
git add crates/chorus-server/src/queue/webhook_dispatch.rs crates/chorus-server/src/queue/mod.rs crates/chorus-server/src/queue/worker.rs crates/chorus-server/Cargo.toml
git commit -m "feat(server): add HMAC-signed webhook dispatcher integrated with worker"
```

---

### Task 9: Lint, format, and create PR for Webhooks

**Step 1: Run clippy and fmt**

```bash
cargo fmt --all
cargo clippy --workspace -- -D warnings
```

**Step 2: Fix any issues and commit**

**Step 3: Create PR**

```bash
git checkout -b feat/webhook-callbacks
git push -u origin feat/webhook-callbacks
gh pr create --title "feat(server): webhook callbacks for delivery status" \
  --body "Closes #16" --label "enhancement,server" --assignee "MrBT-nano"
```

---

## PR 3: Batch Send Endpoints (#17)

### Task 10: Add batch send route for SMS

**Files:**
- Create: `crates/chorus-server/src/routes/batch.rs`
- Modify: `crates/chorus-server/src/routes/mod.rs`
- Modify: `crates/chorus-server/src/app.rs`

**Step 1: Create batch route module**

In `crates/chorus-server/src/routes/batch.rs`:

```rust
use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use crate::app::AppState;
use crate::auth::api_key::AccountContext;
use crate::db::NewMessage;
use crate::queue::SendJob;

/// Maximum recipients per batch request.
const MAX_BATCH_SIZE: usize = 100;

/// A single SMS recipient in a batch.
#[derive(Deserialize)]
pub struct SmsBatchRecipient {
    pub to: String,
    pub body: String,
}

/// SMS batch send request.
#[derive(Deserialize)]
pub struct SendSmsBatchRequest {
    pub from: Option<String>,
    pub recipients: Vec<SmsBatchRecipient>,
}

/// A single email recipient in a batch.
#[derive(Deserialize)]
pub struct EmailBatchRecipient {
    pub to: String,
    pub subject: String,
    pub body: String,
}

/// Email batch send request.
#[derive(Deserialize)]
pub struct SendEmailBatchRequest {
    pub from: Option<String>,
    pub recipients: Vec<EmailBatchRecipient>,
}

/// One message result in the batch response.
#[derive(Serialize)]
pub struct BatchMessageResult {
    pub message_id: Uuid,
    pub to: String,
    pub status: &'static str,
}

/// Batch send response.
#[derive(Serialize)]
pub struct BatchSendResponse {
    pub messages: Vec<BatchMessageResult>,
}

/// Queue a batch of SMS messages. Returns 202 Accepted.
pub async fn send_sms_batch(
    State(state): State<Arc<AppState>>,
    ctx: AccountContext,
    Json(req): Json<SendSmsBatchRequest>,
) -> Result<(StatusCode, Json<BatchSendResponse>), (StatusCode, String)> {
    if req.recipients.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "recipients cannot be empty".into()));
    }
    if req.recipients.len() > MAX_BATCH_SIZE {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("max {MAX_BATCH_SIZE} recipients per batch"),
        ));
    }

    let mut results = Vec::with_capacity(req.recipients.len());

    for recipient in &req.recipients {
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

        let message = state
            .message_repo()
            .insert(&new_msg)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

        let job = SendJob {
            message_id: message.id,
            account_id: message.account_id,
            channel: "sms".into(),
            environment: message.environment.clone(),
            attempt: 0,
        };
        crate::queue::enqueue::enqueue_job(&state, &job)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

        results.push(BatchMessageResult {
            message_id: message.id,
            to: recipient.to.clone(),
            status: "queued",
        });
    }

    Ok((
        StatusCode::ACCEPTED,
        Json(BatchSendResponse { messages: results }),
    ))
}

/// Queue a batch of email messages. Returns 202 Accepted.
pub async fn send_email_batch(
    State(state): State<Arc<AppState>>,
    ctx: AccountContext,
    Json(req): Json<SendEmailBatchRequest>,
) -> Result<(StatusCode, Json<BatchSendResponse>), (StatusCode, String)> {
    if req.recipients.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "recipients cannot be empty".into()));
    }
    if req.recipients.len() > MAX_BATCH_SIZE {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("max {MAX_BATCH_SIZE} recipients per batch"),
        ));
    }

    let mut results = Vec::with_capacity(req.recipients.len());

    for recipient in &req.recipients {
        let new_msg = NewMessage {
            account_id: ctx.account_id,
            api_key_id: ctx.key_id,
            channel: "email".into(),
            sender: req.from.clone(),
            recipient: recipient.to.clone(),
            subject: Some(recipient.subject.clone()),
            body: recipient.body.clone(),
            environment: ctx.environment.clone(),
        };

        let message = state
            .message_repo()
            .insert(&new_msg)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

        let job = SendJob {
            message_id: message.id,
            account_id: message.account_id,
            channel: "email".into(),
            environment: message.environment.clone(),
            attempt: 0,
        };
        crate::queue::enqueue::enqueue_job(&state, &job)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

        results.push(BatchMessageResult {
            message_id: message.id,
            to: recipient.to.clone(),
            status: "queued",
        });
    }

    Ok((
        StatusCode::ACCEPTED,
        Json(BatchSendResponse { messages: results }),
    ))
}
```

**Step 2: Wire into router**

Add `pub mod batch;` to `routes/mod.rs`.

Add routes to `create_router` in `app.rs`:

```rust
        .route("/v1/sms/send-batch", post(routes::batch::send_sms_batch))
        .route("/v1/email/send-batch", post(routes::batch::send_email_batch))
```

**Step 3: Run check**

Run: `cargo check --workspace`
Expected: PASS

**Step 4: Commit**

```bash
git add crates/chorus-server/src/routes/batch.rs crates/chorus-server/src/routes/mod.rs crates/chorus-server/src/app.rs
git commit -m "feat(server): add batch send endpoints for SMS and Email"
```

---

### Task 11: Lint, format, and create PR for Batch Send

**Step 1: Run clippy, fmt, and tests**

```bash
cargo fmt --all
cargo clippy --workspace -- -D warnings
cargo test --workspace
```

**Step 2: Fix any issues and commit**

**Step 3: Create PR**

```bash
git checkout -b feat/batch-send
git push -u origin feat/batch-send
gh pr create --title "feat(server): batch send endpoints for SMS and Email" \
  --body "Closes #17" --label "enhancement,server" --assignee "MrBT-nano"
```
