# Missing Prometheus Metrics Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add 7 new Prometheus metrics + fix 1 existing metric to give Strata rich observability data for Chorus.

**Architecture:** Axum middleware layer for HTTP metrics, manual instrumentation in worker/queue/DB code. Uses `metrics` crate v0.24 macros (`histogram!`, `counter!`, `gauge!`).

**Tech Stack:** Rust, Axum, `metrics` crate, `metrics-exporter-prometheus`, `tower` middleware

---

### Task 1: Create HTTP Metrics Middleware

**Files:**
- Create: `services/chorus-server/src/middleware/mod.rs`
- Create: `services/chorus-server/src/middleware/metrics.rs`
- Modify: `services/chorus-server/src/lib.rs`

**Step 1: Create middleware module declaration**

Create `services/chorus-server/src/middleware/mod.rs`:
```rust
pub mod metrics;
```

**Step 2: Create the metrics middleware**

Create `services/chorus-server/src/middleware/metrics.rs`:
```rust
use axum::extract::MatchedPath;
use axum::middleware::Next;
use axum::response::IntoResponse;
use std::time::Instant;

/// Middleware that records HTTP request duration and total count as Prometheus metrics.
pub async fn track(request: axum::extract::Request, next: Next) -> impl IntoResponse {
    let path = request
        .extensions()
        .get::<MatchedPath>()
        .map(|p| p.as_str().to_owned())
        .unwrap_or_else(|| request.uri().path().to_owned());
    let method = request.method().to_string();

    let start = Instant::now();
    let response = next.run(request).await;
    let duration = start.elapsed().as_secs_f64();
    let status = response.status().as_u16().to_string();

    metrics::histogram!(
        "chorus_http_request_duration_seconds",
        "method" => method.clone(),
        "path" => path.clone(),
        "status" => status.clone(),
    )
    .record(duration);

    metrics::counter!(
        "chorus_http_requests_total",
        "method" => method,
        "path" => path,
        "status" => status,
    )
    .increment(1);

    response
}
```

**Step 3: Export middleware module in lib.rs**

In `services/chorus-server/src/lib.rs`, add:
```rust
pub mod middleware;
```

**Step 4: Run `cargo check -p chorus-server`**

Expected: compiles cleanly, no errors.

**Step 5: Commit**

```bash
git add services/chorus-server/src/middleware/ services/chorus-server/src/lib.rs
git commit -m "feat(server): add HTTP metrics middleware (request duration + count)"
```

---

### Task 2: Wire Middleware into Router

**Files:**
- Modify: `services/chorus-server/src/app.rs:128-193`

**Step 1: Add middleware layer to router**

In `services/chorus-server/src/app.rs`, modify `create_router_with_metrics`:

Add import at top:
```rust
use axum::middleware as axum_middleware;
```

After the `.with_state(state)` call (line 182), add the middleware layer:
```rust
    .layer(axum_middleware::from_fn(crate::middleware::metrics::track))
```

The router section should become:
```rust
    let mut router = Router::new()
        // ... all routes ...
        .with_state(state)
        .layer(axum_middleware::from_fn(crate::middleware::metrics::track));
```

**Step 2: Run `cargo check -p chorus-server`**

Expected: compiles cleanly.

**Step 3: Run `cargo test -p chorus-server`**

Expected: all existing tests pass (middleware is transparent).

**Step 4: Commit**

```bash
git add services/chorus-server/src/app.rs
git commit -m "feat(server): wire HTTP metrics middleware into router"
```

---

### Task 3: Add Worker Active Gauge

**Files:**
- Modify: `services/chorus-server/src/queue/worker.rs:26-221`

**Step 1: Add worker gauge instrumentation**

In `services/chorus-server/src/queue/worker.rs`, modify `process_next_job` to track active workers.

At the start of `process_next_job` function (after line 26), add:
```rust
    metrics::gauge!("chorus_worker_active").increment(1.0);
```

At the end — before every `return Ok(())` AND at the function's final `Ok(())` — we need decrement. The cleanest way is a guard struct. Add this at the top of the file (after imports):

```rust
/// RAII guard that decrements the worker-active gauge when dropped.
struct WorkerGuard;

impl Drop for WorkerGuard {
    fn drop(&mut self) {
        metrics::gauge!("chorus_worker_active").decrement(1.0);
    }
}
```

Then at the start of `process_next_job`, right after the gauge increment:
```rust
    metrics::gauge!("chorus_worker_active").increment(1.0);
    let _guard = WorkerGuard;
```

**Step 2: Run `cargo check -p chorus-server`**

Expected: compiles cleanly.

**Step 3: Commit**

```bash
git add services/chorus-server/src/queue/worker.rs
git commit -m "feat(server): add chorus_worker_active gauge with RAII guard"
```

---

### Task 4: Add Provider Latency Histogram + Fix Provider Error Label

**Files:**
- Modify: `services/chorus-server/src/queue/worker.rs:112-217`

**Step 1: Add provider latency timing around send calls**

Add `use std::time::Instant;` to imports.

Wrap the send call (lines 112-132) with timing:
```rust
    // Send via chorus-core
    let send_start = Instant::now();
    let send_result = match job.channel.as_str() {
        // ... existing match arms unchanged ...
    };
    let send_duration = send_start.elapsed().as_secs_f64();
```

**Step 2: Record provider latency on success**

In the `Ok(result)` arm (around line 135), add after the existing metrics counter:
```rust
            metrics::histogram!(
                "chorus_provider_latency_seconds",
                "channel" => job.channel.clone(),
                "provider" => result.provider.clone(),
            )
            .record(send_duration);
```

**Step 3: Fix provider error label + record error latency**

In the `Err(e)` arm (around line 192), change the existing counter from:
```rust
            metrics::counter!("chorus_provider_errors_total", "channel" => job.channel.clone())
                .increment(1);
```
to:
```rust
            metrics::counter!(
                "chorus_provider_errors_total",
                "channel" => job.channel.clone(),
                "provider" => "unknown",
            )
            .increment(1);

            metrics::histogram!(
                "chorus_provider_latency_seconds",
                "channel" => job.channel.clone(),
                "provider" => "unknown",
            )
            .record(send_duration);
```

**Step 4: Run `cargo check -p chorus-server`**

Expected: compiles cleanly.

**Step 5: Commit**

```bash
git add services/chorus-server/src/queue/worker.rs
git commit -m "feat(server): add provider latency histogram + fix provider error label"
```

---

### Task 5: Add Cost Metric

**Files:**
- Modify: `services/chorus-server/src/queue/worker.rs:135-191`

**Step 1: Add cost counter after successful delivery**

In the `Ok(result)` arm, after the existing `chorus_messages_total` counter (line 173), add:
```rust
            // Track cost — use channel-based estimate in microdollars
            // SMS: ~7500 microdollars ($0.0075), Email: ~1000 microdollars ($0.001)
            let cost = match job.channel.as_str() {
                "sms" => 7500_u64,
                "email" => 1000_u64,
                _ => 0,
            };
            metrics::counter!(
                "chorus_message_cost_microdollars_total",
                "channel" => job.channel.clone(),
                "provider" => result.provider.clone(),
            )
            .increment(cost);
```

**Step 2: Run `cargo check -p chorus-server`**

Expected: compiles cleanly.

**Step 3: Commit**

```bash
git add services/chorus-server/src/queue/worker.rs
git commit -m "feat(server): add chorus_message_cost_microdollars_total counter"
```

---

### Task 6: Add DB Query Duration Metrics

**Files:**
- Modify: `services/chorus-server/src/db/postgres.rs`

**Step 1: Add timing to key DB methods**

Add `use std::time::Instant;` to imports.

Create a helper macro at the top of the file (after imports):
```rust
/// Record DB query duration for a given operation.
macro_rules! record_db_duration {
    ($op:expr, $start:expr) => {
        metrics::histogram!(
            "chorus_db_query_duration_seconds",
            "operation" => $op,
        )
        .record($start.elapsed().as_secs_f64());
    };
}
```

Instrument the most critical methods (high-frequency in worker loop):

For `find_by_id` (line 86):
```rust
    async fn find_by_id(&self, id: Uuid, account_id: Uuid) -> Result<Option<Message>, DbError> {
        let start = Instant::now();
        let msg = sqlx::query_as::<_, Message>(
            "SELECT * FROM messages WHERE id = $1 AND account_id = $2",
        )
        .bind(id)
        .bind(account_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| DbError::Internal(e.into()))?;
        record_db_duration!("find_by_id", start);
        Ok(msg)
    }
```

For `update_status` (line 119):
```rust
    async fn update_status(
        &self,
        id: Uuid,
        status: &str,
        provider: Option<&str>,
        provider_message_id: Option<&str>,
        error_message: Option<&str>,
    ) -> Result<(), DbError> {
        let start = Instant::now();
        sqlx::query(
            "UPDATE messages SET status = $1, provider = $2, provider_message_id = $3,
             error_message = $4, attempts = attempts + 1,
             delivered_at = CASE WHEN $1 = 'delivered' THEN now() ELSE delivered_at END
             WHERE id = $5",
        )
        .bind(status)
        .bind(provider)
        .bind(provider_message_id)
        .bind(error_message)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(|e| DbError::Internal(e.into()))?;
        record_db_duration!("update_status", start);
        Ok(())
    }
```

For `insert_delivery_event` (line 145):
```rust
    async fn insert_delivery_event(
        &self,
        message_id: Uuid,
        status: &str,
        provider_data: Option<serde_json::Value>,
    ) -> Result<(), DbError> {
        let start = Instant::now();
        sqlx::query(
            "INSERT INTO delivery_events (message_id, status, provider_data)
             VALUES ($1, $2, $3)",
        )
        .bind(message_id)
        .bind(status)
        .bind(provider_data)
        .execute(&self.pool)
        .await
        .map_err(|e| DbError::Internal(e.into()))?;
        record_db_duration!("insert_delivery_event", start);
        Ok(())
    }
```

For `insert` (message, line 65):
```rust
    async fn insert(&self, msg: &NewMessage) -> Result<Message, DbError> {
        let start = Instant::now();
        let message = sqlx::query_as::<_, Message>(
            "INSERT INTO messages (account_id, api_key_id, channel, sender, recipient, subject, body, environment)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
             RETURNING *",
        )
        .bind(msg.account_id)
        .bind(msg.api_key_id)
        .bind(&msg.channel)
        .bind(&msg.sender)
        .bind(&msg.recipient)
        .bind(&msg.subject)
        .bind(&msg.body)
        .bind(&msg.environment)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| DbError::Internal(e.into()))?;
        record_db_duration!("insert_message", start);
        Ok(message)
    }
```

**Step 2: Run `cargo check -p chorus-server`**

Expected: compiles cleanly.

**Step 3: Commit**

```bash
git add services/chorus-server/src/db/postgres.rs
git commit -m "feat(server): add chorus_db_query_duration_seconds histogram"
```

---

### Task 7: Add Redis Operation Duration Metrics

**Files:**
- Modify: `services/chorus-server/src/queue/enqueue.rs`
- Modify: `services/chorus-server/src/queue/delayed.rs`
- Modify: `services/chorus-server/src/queue/dead_letter.rs`
- Modify: `services/chorus-server/src/queue/worker.rs` (BRPOP timing)

**Step 1: Add timing macro to queue/mod.rs**

In `services/chorus-server/src/queue/mod.rs`, add at the end:
```rust
/// Record Redis operation duration.
macro_rules! record_redis_duration {
    ($op:expr, $start:expr) => {
        metrics::histogram!(
            "chorus_redis_operation_duration_seconds",
            "operation" => $op,
        )
        .record($start.elapsed().as_secs_f64());
    };
}
pub(crate) use record_redis_duration;
```

**Step 2: Instrument BRPOP in worker.rs**

In `process_next_job` (worker.rs), wrap the BRPOP call:
```rust
    let brpop_start = std::time::Instant::now();
    let result: Option<(String, String)> = redis::cmd("BRPOP")
        .arg(super::QUEUE_KEY)
        .arg(5)
        .query_async(&mut conn)
        .await?;
    super::record_redis_duration!("brpop", brpop_start);
```

**Step 3: Instrument LPUSH in enqueue.rs**

Add `use std::time::Instant;` and wrap:
```rust
pub async fn job(state: &AppState, job: &super::SendJob) -> anyhow::Result<()> {
    let mut conn = state.redis.get_multiplexed_tokio_connection().await?;
    let payload = serde_json::to_string(job)?;
    let start = Instant::now();
    redis::cmd("LPUSH")
        .arg(super::QUEUE_KEY)
        .arg(payload)
        .query_async::<i64>(&mut conn)
        .await?;
    super::record_redis_duration!("lpush", start);
    Ok(())
}
```

**Step 4: Instrument delayed.rs (ZADD, ZRANGEBYSCORE, LPUSH)**

Add `use std::time::Instant;` and wrap each Redis call.

In `schedule_retry`:
```rust
    let start = Instant::now();
    redis::cmd("ZADD")
        .arg(super::DELAYED_KEY)
        .arg(retry_at)
        .arg(payload)
        .query_async::<i64>(&mut conn)
        .await?;
    super::record_redis_duration!("zadd", start);
```

In `poll_delayed_jobs`, wrap ZRANGEBYSCORE:
```rust
    let start = Instant::now();
    let jobs: Vec<String> = redis::cmd("ZRANGEBYSCORE")
        .arg(super::DELAYED_KEY)
        .arg("-inf")
        .arg(now)
        .query_async(&mut conn)
        .await?;
    super::record_redis_duration!("zrangebyscore", start);
```

**Step 5: Instrument dead_letter.rs (LPUSH)**

Add `use std::time::Instant;` and wrap:
```rust
    let start = Instant::now();
    redis::cmd("LPUSH")
        .arg(super::DEAD_LETTER_KEY)
        .arg(payload)
        .query_async::<i64>(&mut conn)
        .await?;
    super::record_redis_duration!("lpush_dlq", start);
```

**Step 6: Run `cargo check -p chorus-server`**

Expected: compiles cleanly.

**Step 7: Run `cargo test -p chorus-server`**

Expected: all tests pass.

**Step 8: Commit**

```bash
git add services/chorus-server/src/queue/
git commit -m "feat(server): add chorus_redis_operation_duration_seconds histogram"
```

---

### Task 8: Verify All Metrics + Lint + Format

**Files:** None new — verification only.

**Step 1: Run full test suite**

```bash
cargo test --workspace
```
Expected: all tests pass.

**Step 2: Run clippy**

```bash
cargo clippy --workspace -- -D warnings
```
Expected: no warnings.

**Step 3: Run fmt**

```bash
cargo fmt --all
```

**Step 4: Run cargo deny**

```bash
cargo deny check
```
Expected: passes (no new deps added).

**Step 5: Commit any formatting fixes**

```bash
git add -A
git commit -m "style: format metrics instrumentation code"
```

(Skip if no changes.)

---

### Task 9: Update GitHub Issue

**Step 1: Close issue #33 with a comment**

```bash
gh issue close 33 -R cntm-labs/chorus --comment "Implemented in commits on main. Added 7 new metrics + fixed provider label:
- chorus_http_request_duration_seconds (histogram)
- chorus_http_requests_total (counter)
- chorus_provider_latency_seconds (histogram)
- chorus_db_query_duration_seconds (histogram)
- chorus_redis_operation_duration_seconds (histogram)
- chorus_worker_active (gauge)
- chorus_message_cost_microdollars_total (counter)
- Fixed: chorus_provider_errors_total now includes provider label"
```

---

## Summary of New Metrics

| # | Metric | Type | Labels |
|---|--------|------|--------|
| 1 | `chorus_http_request_duration_seconds` | Histogram | method, path, status |
| 2 | `chorus_http_requests_total` | Counter | method, path, status |
| 3 | `chorus_provider_latency_seconds` | Histogram | channel, provider |
| 4 | `chorus_db_query_duration_seconds` | Histogram | operation |
| 5 | `chorus_redis_operation_duration_seconds` | Histogram | operation |
| 6 | `chorus_worker_active` | Gauge | (none) |
| 7 | `chorus_message_cost_microdollars_total` | Counter | channel, provider |
| Fix | `chorus_provider_errors_total` | Counter | channel, **provider** (added) |
