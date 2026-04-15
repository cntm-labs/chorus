# Design: Add Missing Prometheus Metrics (#33)

## Context

Chorus v0.3.0 exposes 5 basic metrics (4 counters + 1 gauge). This is insufficient for Strata to build meaningful dashboards. We need richer instrumentation: HTTP latency, per-provider breakdown, infrastructure timing, worker health, and cost tracking.

## Metrics to Add

### New Metrics (7)

| Metric | Type | Labels | Location |
|--------|------|--------|----------|
| `chorus_http_request_duration_seconds` | Histogram | `method`, `path`, `status` | Axum middleware layer |
| `chorus_http_requests_total` | Counter | `method`, `path`, `status` | Axum middleware layer |
| `chorus_provider_latency_seconds` | Histogram | `channel`, `provider` | worker.rs (success path) |
| `chorus_db_query_duration_seconds` | Histogram | `operation` | DB repo wrapper |
| `chorus_redis_operation_duration_seconds` | Histogram | `operation` | Queue module |
| `chorus_worker_active` | Gauge | (none) | worker.rs |
| `chorus_message_cost_microdollars_total` | Counter | `channel`, `provider` | worker.rs (after delivery) |

### Fix Existing (1)

- `chorus_provider_errors_total` — add `provider` label (use `"unknown"` when provider info unavailable from chorus-core error)

## Architecture

### HTTP Metrics Middleware

New module `middleware/metrics.rs`:
- Custom Axum middleware layer wrapping all routes
- Uses `axum::extract::MatchedPath` for path normalization (`/v1/messages/{id}` not `/v1/messages/abc123`) to prevent label cardinality explosion
- Records both histogram (latency) and counter (total requests) per request

### Worker Instrumentation

In `queue/worker.rs`:
- `chorus_worker_active` gauge: increment before `process_next_job`, decrement after (using RAII guard pattern)
- `chorus_provider_latency_seconds`: wrap `router.send_sms/send_email` call with `Instant::now()`
- `chorus_message_cost_microdollars_total`: increment after successful delivery using cost from billing usage

### DB Query Timing

In `db/postgres.rs`:
- Wrap key repo methods with `Instant::now()` timing
- Label `operation`: `find_by_id`, `update_status`, `insert_delivery_event`, `list_by_account_channel`

### Redis Operation Timing

In `queue/mod.rs` and related files:
- Wrap Redis commands (`BRPOP`, `LPUSH`, `ZADD`, `ZRANGEBYSCORE`) with timing
- Label `operation`: `brpop`, `lpush`, `zadd`, `zrangebyscore`

## Histogram Buckets

Use `metrics-exporter-prometheus` default buckets: 5ms, 10ms, 25ms, 50ms, 100ms, 250ms, 500ms, 1s, 2.5s, 5s, 10s.

## Files to Modify

| File | Change |
|------|--------|
| `services/chorus-server/src/middleware/mod.rs` | NEW — module declaration |
| `services/chorus-server/src/middleware/metrics.rs` | NEW — HTTP metrics middleware |
| `services/chorus-server/src/app.rs` | Add middleware layer to router |
| `services/chorus-server/src/queue/worker.rs` | Provider latency, worker gauge, cost metric, fix provider label |
| `services/chorus-server/src/queue/mod.rs` | Redis operation timing |
| `services/chorus-server/src/db/postgres.rs` | DB query timing |
| `services/chorus-server/src/lib.rs` | Export middleware module |

## Not In Scope

- OpenTelemetry traces — handled by #40 (structured logging)
- Provider name in chorus-core ChorusError — future enhancement, use "unknown" for now

## Verification

1. `cargo check --workspace` — compiles cleanly
2. `cargo test --workspace` — all tests pass
3. `cargo clippy --workspace -- -D warnings` — no warnings
4. Start server locally, hit endpoints, verify `/metrics` shows all new metrics
5. Confirm histogram buckets appear correctly in Prometheus text format

## Strata Dependencies

These metrics unblock:
- cntm-labs/strata#3 (dashboard template)
- cntm-labs/strata#4 (throughput panel)
- cntm-labs/strata#5 (provider health)
- cntm-labs/strata#6 (queue depth — already works)
