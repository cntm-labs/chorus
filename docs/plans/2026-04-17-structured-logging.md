# Structured Logging for Loki Integration Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Enrich Chorus log output with structured fields (request_id, account_id, message_id, channel, provider) so Strata/Loki can filter and correlate logs effectively.

**Architecture:** Add request ID middleware that generates a UUID per HTTP request and injects it into the tracing span. Enrich worker processing spans with message context fields. All structured fields are automatically included in JSON log output.

**Tech Stack:** Rust, Axum, `tracing`, `tracing-subscriber`, `uuid`

**Closes:** #40

---

### Task 1: Add Request ID Middleware

**Files:**
- Create: `services/chorus-server/src/middleware/request_id.rs`
- Modify: `services/chorus-server/src/middleware/mod.rs`
- Modify: `services/chorus-server/src/app.rs`

**Step 1: Create request ID middleware**

Create `services/chorus-server/src/middleware/request_id.rs`:
```rust
use axum::middleware::Next;
use axum::response::IntoResponse;
use uuid::Uuid;

/// Middleware that generates a unique request ID and wraps the request in a tracing span.
pub async fn inject(request: axum::extract::Request, next: Next) -> impl IntoResponse {
    let request_id = request
        .headers()
        .get("x-request-id")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_owned())
        .unwrap_or_else(|| Uuid::now_v7().to_string());

    let span = tracing::info_span!(
        "request",
        request_id = %request_id,
        method = %request.method(),
        path = %request.uri().path(),
    );

    let response = {
        let _guard = span.enter();
        next.run(request).await
    };

    let mut response = response;
    response.headers_mut().insert(
        "x-request-id",
        request_id.parse().expect("valid header value"),
    );
    response
}
```

**Step 2: Export module**

In `services/chorus-server/src/middleware/mod.rs`, add:
```rust
pub mod request_id;
```

**Step 3: Wire into router**

In `app.rs`, add the request_id middleware layer *before* the metrics middleware (outermost layer runs first):
```rust
    .layer(axum_middleware::from_fn(crate::middleware::metrics::track))
    .layer(axum_middleware::from_fn(crate::middleware::request_id::inject));
```

Note: layers are applied in reverse order — the last `.layer()` is the outermost. So request_id should be added after metrics to run first.

**Step 4: Run `cargo check -p chorus-server`**

Expected: compiles cleanly.

**Step 5: Run `cargo test -p chorus-server`**

Expected: all tests pass. Verify `x-request-id` header is present in responses.

**Step 6: Commit**

```bash
git add services/chorus-server/src/middleware/ services/chorus-server/src/app.rs
git commit -m "feat(server): add request ID middleware for Loki correlation"
```

---

### Task 2: Enrich Worker Spans with Message Context

**Files:**
- Modify: `services/chorus-server/src/queue/worker.rs`

**Step 1: Add tracing span to process_next_job**

After the job is deserialized (line where `let job: super::SendJob = ...`), wrap the remainder of the function in a tracing span:

```rust
    let job: super::SendJob = serde_json::from_str(&payload)?;

    let span = tracing::info_span!(
        "process_job",
        message_id = %job.message_id,
        account_id = %job.account_id,
        channel = %job.channel,
        environment = %job.environment,
        attempt = job.attempt,
    );
    let _guard = span.enter();
```

**Step 2: Add provider field to span on successful send**

After the send result is known, record the provider:
```rust
        Ok(result) => {
            tracing::Span::current().record("provider", &result.provider.as_str());
            // ... existing code
        }
```

Note: for this to work, the span must declare provider as an Empty field:
```rust
    let span = tracing::info_span!(
        "process_job",
        message_id = %job.message_id,
        account_id = %job.account_id,
        channel = %job.channel,
        environment = %job.environment,
        attempt = job.attempt,
        provider = tracing::field::Empty,
    );
```

**Step 3: Replace raw tracing calls with structured fields**

The existing `tracing::info!()` and `tracing::error!()` calls in worker.rs should now automatically inherit span context. No changes needed — the span fields propagate to child events.

**Step 4: Run `cargo check -p chorus-server`**

Expected: compiles cleanly.

**Step 5: Commit**

```bash
git add services/chorus-server/src/queue/worker.rs
git commit -m "feat(server): enrich worker spans with message context fields"
```

---

### Task 3: Enrich Worker Startup and Error Spans

**Files:**
- Modify: `services/chorus-server/src/queue/worker.rs`

**Step 1: Add worker ID to spawn loop span**

In `spawn_workers`, wrap each worker loop in a span:
```rust
pub fn spawn_workers(state: Arc<AppState>, config: Arc<Config>, concurrency: usize) {
    for i in 0..concurrency {
        let state = Arc::clone(&state);
        let config = Arc::clone(&config);
        tokio::spawn(async move {
            let span = tracing::info_span!("worker", worker_id = i);
            let _guard = span.enter();
            tracing::info!("queue worker started");
            loop {
                if let Err(e) = process_next_job(&state, &config).await {
                    tracing::error!(error = %e, "worker error");
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                }
            }
        });
    }
}
```

**Step 2: Run `cargo check -p chorus-server`**

**Step 3: Commit**

```bash
git add services/chorus-server/src/queue/worker.rs
git commit -m "feat(server): add worker_id span to worker loops"
```

---

### Task 4: Enrich Webhook Dispatch Spans

**Files:**
- Modify: `services/chorus-server/src/queue/webhook_dispatch.rs`

**Step 1: Add structured fields to webhook dispatch**

In `dispatch_webhooks`, add span:
```rust
pub async fn dispatch_webhooks(
    state: &Arc<AppState>,
    account_id: Uuid,
    event: &str,
    payload: &WebhookPayload,
) {
    let span = tracing::info_span!(
        "dispatch_webhooks",
        account_id = %account_id,
        event = %event,
        message_id = %payload.message_id,
    );
    let _guard = span.enter();
    // ... existing code
}
```

In `deliver_webhook`, add span:
```rust
async fn deliver_webhook(url: &str, secret: &str, event: &str, body: &str) -> bool {
    let span = tracing::info_span!("deliver_webhook", url = %url, event = %event);
    let _guard = span.enter();
    // ... existing code
}
```

**Step 2: Run `cargo check -p chorus-server`**

**Step 3: Commit**

```bash
git add services/chorus-server/src/queue/webhook_dispatch.rs
git commit -m "feat(server): add structured spans to webhook dispatch"
```

---

### Task 5: Add Structured Fields to Delayed Queue Poller

**Files:**
- Modify: `services/chorus-server/src/queue/delayed.rs`

**Step 1: Add span to poll_delayed_jobs**

```rust
async fn poll_delayed_jobs(redis: &redis::Client) -> anyhow::Result<()> {
    let mut conn = redis.get_multiplexed_tokio_connection().await?;
    let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();

    // ... existing ZRANGEBYSCORE code ...

    for payload in jobs {
        let job: super::SendJob = match serde_json::from_str(&payload) {
            Ok(j) => j,
            Err(_) => continue,
        };

        let span = tracing::info_span!(
            "requeue_delayed",
            message_id = %job.message_id,
            account_id = %job.account_id,
            attempt = job.attempt,
        );
        let _guard = span.enter();

        // ... existing ZREM + LPUSH code ...
        tracing::debug!("moved delayed job back to main queue");
    }

    Ok(())
}
```

Note: This changes the loop to parse the job JSON before ZREM, which is fine — we already have the payload string.

**Step 2: Run `cargo check -p chorus-server`**

**Step 3: Commit**

```bash
git add services/chorus-server/src/queue/delayed.rs
git commit -m "feat(server): add structured spans to delayed queue poller"
```

---

### Task 6: Final Verification

**Step 1: Run full CI checks**

```bash
cargo test --workspace
cargo clippy --workspace -- -D warnings
cargo fmt --all
cargo deny check
```

**Step 2: Verify JSON log output**

Manually test with `CHORUS_LOG_FORMAT=json cargo run` (or check that the structured fields appear in log output). Expected fields in JSON logs:
- HTTP requests: `request_id`, `method`, `path`
- Worker processing: `message_id`, `account_id`, `channel`, `provider`, `attempt`, `worker_id`
- Webhook dispatch: `account_id`, `event`, `message_id`, `url`

**Step 3: Commit any fixes**

```bash
git add -A && git commit -m "style: format structured logging code"
```

---

## Summary of Changes

| Task | Component | Structured Fields Added |
|------|-----------|----------------------|
| 1 | Request ID middleware | request_id, method, path + X-Request-Id header |
| 2 | Worker job processing | message_id, account_id, channel, environment, attempt, provider |
| 3 | Worker startup | worker_id |
| 4 | Webhook dispatch | account_id, event, message_id, url |
| 5 | Delayed queue poller | message_id, account_id, attempt |
| 6 | Verification | Full CI + JSON log verification |
