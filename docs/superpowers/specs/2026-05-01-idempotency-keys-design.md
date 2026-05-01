# Idempotency Keys — Design

**Status:** Approved
**Date:** 2026-05-01
**Author:** chorus team
**Closes:** roadmap item C1 (production-safety tier, second deliverable after suppression list)

---

## 1. Goal

Prevent Chorus from creating duplicate messages when a client retries the same request, by supporting the industry-standard `Idempotency-Key` HTTP header (Stripe-compatible). The first call performs the action; subsequent calls within 24 hours return the **byte-for-byte identical response** without re-executing side effects.

This is the second deliverable in the **production-safety** tier (after the suppression list MVP). It directly addresses the highest-risk failure mode for paid CPaaS traffic: a network blip that makes a client retry, charging the customer twice and delivering the same message twice.

## 2. Scope

### In scope
- HTTP header `Idempotency-Key` (1–255 chars, ASCII-printable; spaces allowed) — **opt-in**: requests without the header keep current behavior (full backward compatibility).
- Scope: `(api_key_id, idempotency_key)` — each API key has its own namespace; rotating a key clears its history (security feature).
- TTL: 24 hours from `created_at`.
- Conflict policy: same key + different request body → `422 idempotency_key_reused`. Body comparison is SHA-256 of raw bytes (no JSON canonicalization).
- In-flight retry handling: row-lock + wait via `SELECT ... FOR UPDATE`; stale-lock recovery after 60 s; `statement_timeout = 5 s` to bound waits.
- Six send routes: `/v1/sms`, `/v1/email`, `/v1/messages`, `/v1/sms/batch`, `/v1/email/batch`, `/v1/otp`.
- Response cache: stores HTTP status + raw response body bytes for identical replay.
- Cleanup: in-process tokio task deleting expired rows every 5 minutes.

### Out of scope (deferred to follow-up specs — see §8)
- `/v1/billing/*` (Stripe has its own idempotency layer; chain via follow-up).
- `/v1/webhooks`, `/v1/keys`, `/v1/suppressions` (low risk or naturally idempotent).
- Valkey cache layer (add when metrics justify; see §8 trigger criteria).
- Per-recipient idempotency inside a batch (MVP uses batch-as-a-whole).
- Distributed locks across chorus-server instances (Postgres row locks already work cross-instance).
- Renaming "Redis" → "Valkey" in project docs (separate cleanup PR; not coupled to this feature).

## 3. Schema

New migration `services/chorus-server/src/db/migrations/008_create_idempotency_keys.sql`:

```sql
CREATE TABLE idempotency_keys (
    api_key_id        UUID NOT NULL REFERENCES api_keys(id) ON DELETE CASCADE,
    idempotency_key   TEXT NOT NULL CHECK (length(idempotency_key) BETWEEN 1 AND 255),
    request_hash      BYTEA NOT NULL,
    request_method    TEXT NOT NULL,
    request_path      TEXT NOT NULL,
    status            TEXT NOT NULL CHECK (status IN ('in_progress', 'completed')),
    response_status   SMALLINT,
    response_body     BYTEA,
    created_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at        TIMESTAMPTZ NOT NULL DEFAULT (now() + interval '24 hours'),
    PRIMARY KEY (api_key_id, idempotency_key)
);

CREATE INDEX idempotency_keys_expires_at_idx ON idempotency_keys (expires_at);
```

### Design notes

| Column | Reason |
|---|---|
| `api_key_id` (FK + cascade) | Rotating/deleting an API key clears its idempotency history automatically. |
| `idempotency_key` TEXT | Stripe convention; UUID, random, or timestamp-based all accepted. |
| `request_hash` BYTEA(32) | SHA-256 of raw request body bytes; BYTEA compares faster than TEXT. |
| `request_method` + `request_path` | Defends against the same key being reused across endpoints — surfaces as a hash mismatch, but stored explicitly for diagnostics. |
| `status` (2-value enum) | `in_progress` on first INSERT; `completed` after `finalize`. Used to detect stale locks. |
| `response_status` SMALLINT | Replay must return the original HTTP status (202 normal; 400/422 for cached validation errors). |
| `response_body` BYTEA | Raw bytes — replay returns identical output (no JSON re-serialization, no key reordering). |
| `expires_at` + index | Enables cleanup via index range scan. |
| **Not stored:** `account_id` | Derivable from `api_keys.account_id`; avoid denormalization. |
| **Not stored:** index on `created_at` | No query pattern uses it. |

### Why BYTEA over JSONB for `response_body`

Stripe's spec requires byte-for-byte replay including whitespace and key ordering. JSONB canonicalizes both, breaking the guarantee. Trade-off accepted: psql debugging needs `convert_from(response_body, 'UTF8')`.

### Capacity estimate

- ~150 bytes overhead + ~500 bytes typical response body = ~650 bytes/row.
- 1k sends/day × 24h TTL → ~1k steady-state rows per API key.
- 1M sends/day → ~620 MB on disk; cleanup keeps it bounded.

## 4. Components

### 4.1 `db::idempotency` (new file `services/chorus-server/src/db/idempotency.rs`)

```rust
#[derive(Debug, Clone)]
pub struct IdempotencyRecord {
    pub api_key_id: Uuid,
    pub idempotency_key: String,
    pub request_hash: [u8; 32],
    pub request_method: String,
    pub request_path: String,
    pub status: IdempotencyStatus,
    pub response_status: Option<u16>,
    pub response_body: Option<Vec<u8>>,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

pub enum IdempotencyStatus { InProgress, Completed }

pub enum LookupOutcome {
    /// First time this key has been seen — caller proceeds and calls `complete`.
    Fresh,
    /// Existing completed row with matching hash — caller returns this response.
    Replay { status: u16, body: Vec<u8> },
    /// Existing row with different hash — caller returns 422.
    HashMismatch,
}

#[async_trait]
pub trait IdempotencyRepository: Send + Sync {
    async fn begin(
        &self,
        api_key_id: Uuid,
        key: &str,
        request_hash: &[u8; 32],
        method: &str,
        path: &str,
    ) -> Result<LookupOutcome, DbError>;

    async fn complete(
        &self,
        api_key_id: Uuid,
        key: &str,
        response_status: u16,
        response_body: &[u8],
    ) -> Result<(), DbError>;

    async fn delete_expired(&self, limit: i64) -> Result<u64, DbError>;
}
```

`begin` is implemented as a single SQL block that performs INSERT-or-lock atomically:

```sql
WITH inserted AS (
    INSERT INTO idempotency_keys (api_key_id, idempotency_key, request_hash,
                                   request_method, request_path, status)
    VALUES ($1, $2, $3, $4, $5, 'in_progress')
    ON CONFLICT (api_key_id, idempotency_key) DO UPDATE
        SET status = 'in_progress',
            request_hash = EXCLUDED.request_hash,
            request_method = EXCLUDED.request_method,
            request_path = EXCLUDED.request_path,
            created_at = now(),
            expires_at = now() + interval '24 hours',
            response_status = NULL,
            response_body = NULL
        WHERE idempotency_keys.status = 'in_progress'
          AND idempotency_keys.created_at < now() - interval '60 seconds'
    RETURNING request_hash, status, response_status, response_body, 'fresh'::text AS outcome
)
SELECT * FROM inserted
UNION ALL
SELECT request_hash, status, response_status, response_body, 'existing'::text
FROM idempotency_keys
WHERE api_key_id = $1 AND idempotency_key = $2
  AND NOT EXISTS (SELECT 1 FROM inserted)
FOR UPDATE;
```

Caller branches on `(outcome, status, hash match)`:
- `outcome='fresh'` → `LookupOutcome::Fresh`
- `outcome='existing'` AND status='completed' AND hash matches → `Replay`
- `outcome='existing'` AND hash differs → `HashMismatch`
- `outcome='existing'` AND status='in_progress' → unreachable in normal flow: `FOR UPDATE` blocks until the holder commits, after which `status` becomes `'completed'`. If this case is observed (e.g. a future change drops `FOR UPDATE`) the repo logs an error and returns `DbError::Inconsistent`.

The connection running this query sets `statement_timeout = '5s'`. If a hung holder causes the wait to exceed 5 s, the query errors with PostgreSQL `57014` (statement_timeout) and the repo returns `DbError::Timeout`. The helper layer (`idempotency::begin`) maps `DbError::Timeout` to `IdempotencyAction::Respond { 503 concurrent_request, Retry-After: 1 }`.

### 4.2 `idempotency.rs` (new file `services/chorus-server/src/idempotency.rs`)

```rust
pub const HEADER_NAME: &str = "Idempotency-Key";
pub const MAX_REQUEST_BODY_BYTES: usize = 1 << 20;   // 1 MiB
pub const MAX_RESPONSE_BODY_BYTES: usize = 1 << 16;  // 64 KiB cached

pub enum IdempotencyAction {
    Proceed { token: IdempotencyToken },
    Respond { status: StatusCode, body: Bytes },
    Skip,
}

pub struct IdempotencyToken {
    api_key_id: Uuid,
    key: String,
}

pub async fn begin(
    state: &AppState,
    api_key_id: Uuid,
    headers: &HeaderMap,
    method: &Method,
    path: &str,
    body_bytes: &[u8],
) -> Result<IdempotencyAction, DbError>;

pub async fn finalize(
    state: &AppState,
    token: IdempotencyToken,
    status: StatusCode,
    body: &[u8],
) -> Result<(), DbError>;

fn is_valid_key(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 255
        && s.chars().all(|c| c.is_ascii_graphic() || c == ' ')
}
```

### 4.3 Route integration pattern

Each instrumented route reads the body as raw `Bytes`, calls `begin` before parsing, then calls `finalize` after building the response:

```rust
pub async fn send_sms(
    State(state): State<Arc<AppState>>,
    ctx: AccountContext,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let action = idempotency::begin(&state, ctx.key_id, &headers, &Method::POST, "/v1/sms", &body).await;
    let token = match action {
        Ok(IdempotencyAction::Skip) => None,
        Ok(IdempotencyAction::Proceed { token }) => Some(token),
        Ok(IdempotencyAction::Respond { status, body }) => return (status, body).into_response(),
        Err(e) => return error_500(e),
    };

    let req: SendSmsRequest = match serde_json::from_slice(&body) { /* ... */ };

    // suppression check + insert message + enqueue (existing logic)
    let response_body = serde_json::to_vec(&SendResponse { /* ... */ }).unwrap();
    let status = StatusCode::ACCEPTED;

    if let Some(token) = token {
        if let Err(e) = idempotency::finalize(&state, token, status, &response_body).await {
            tracing::warn!(error = %e, "idempotency finalize failed");
        }
    }

    (status, response_body).into_response()
}
```

**Why raw `Bytes` instead of `Json<T>`:** the SHA-256 hash must be computed from the raw bytes the client sent, before any deserialization that would lose whitespace and key ordering.

**Failure-mode policy:** if `finalize` fails, the message has already been queued. We log a warning and return the success response. The next retry will encounter a stale `in_progress` row, which the 60-second recovery window will reset.

### 4.4 Cleanup background task

```rust
pub async fn cleanup_loop(state: Arc<AppState>) {
    let mut interval = tokio::time::interval(Duration::from_secs(300));
    loop {
        interval.tick().await;
        match state.idempotency_repo().delete_expired(10_000).await {
            Ok(n) if n > 0 => tracing::info!(deleted = n, "cleaned expired idempotency keys"),
            Ok(_) => {}
            Err(e) => tracing::warn!(error = %e, "idempotency cleanup failed"),
        }
    }
}
```

Spawned in `app.rs` at server startup via `tokio::spawn(cleanup_loop(state.clone()))`. The 10 000-row limit prevents lock contention spikes; backlogs drain over subsequent ticks.

### 4.5 `AppState` wiring

```rust
pub fn idempotency_repo(&self) -> &Arc<dyn IdempotencyRepository> {
    &self.idempotency_repo
}
```

Injected in `AppState::new()` alongside `suppression_repo`.

## 5. Data Flow

### 5.1 Fresh request
```
client → POST /v1/sms
         Headers: Idempotency-Key: abc-123
         Body:   {"to":"+66...", "body":"hi"}
   ↓ AccountContext extractor
   ↓ idempotency::begin → INSERT ON CONFLICT … FOR UPDATE
        → no prior row → status='in_progress', hash=h1
        → outcome = Fresh
   ↓ parse + suppression + insert message + enqueue
   ↓ response: 202 + {"message_id":"…","status":"queued"}
   ↓ idempotency::finalize → status='completed', cache body
   ↓ return 202
```

### 5.2 Replay (post-completion retry)
```
T+0s:   request #1 → 202, cached
T+10s:  retry (network blip)
   ↓ begin → ON CONFLICT DO UPDATE WHERE …(stale)… not satisfied
            → fallback SELECT FOR UPDATE → row found, completed, hash matches
            → outcome = Replay { 202, <bytes> }
   ↓ skip parse, skip suppression, skip insert, skip enqueue
   ↓ return 202 + identical body
```
**Result:** no new message, no duplicate enqueue, no double-charge.

### 5.3 In-flight retry
```
T+0ms:   #1 begins → INSERT in_progress; tx still processing
T+50ms:  #2 arrives → ON CONFLICT DO UPDATE skipped (not stale, only 50 ms old)
            → fallback SELECT FOR UPDATE → BLOCKS on #1's row lock
T+200ms: #1 finalize commits → lock released
T+201ms: #2 acquires lock → row is completed, hash matches
            → outcome = Replay → returns same response as #1
```
If `statement_timeout = 5 s` fires while #2 is waiting → `503 concurrent_request` with `Retry-After: 1`.

### 5.4 Hash mismatch
```
T+0s: POST /v1/sms (key="abc", body={to:"+66A"}) → 202, cached
T+5s: POST /v1/sms (key="abc", body={to:"+66B"}) ← client bug
   ↓ begin → existing row, hash differs
   ↓ outcome = HashMismatch
   ↓ return 422 idempotency_key_reused
```

### 5.5 Stale lock recovery
```
T+0s:    #1 → INSERT in_progress → server crashes mid-processing
T+90s:   #2 with same key arrives
   ↓ ON CONFLICT DO UPDATE WHERE status='in_progress' AND created_at < now()-60s
   ↓ condition satisfied → row reset to fresh in_progress with new hash
   ↓ outcome = Fresh → proceeds normally
```

## 6. Error Handling

| Situation | HTTP | Body |
|---|---|---|
| No `Idempotency-Key` header | (unchanged) | Existing behavior. |
| Header empty / >255 chars / non-ASCII | `400` | `{error:{code:"invalid_idempotency_key", message:"..."}}` |
| Header valid, key new, processed normally | (existing: 202/400/422 etc.) | Normal response, then cached. |
| Header valid, key seen, hash matches, completed | (status from first call) | Identical bytes from first call. |
| Header valid, key seen, hash differs | `422` | `{error:{code:"idempotency_key_reused", message:"..."}}` |
| Header valid, key seen, in-flight (lock acquired before timeout) | (status from #1 after wait) | Response of #1. |
| Header valid, key seen, lock wait > 5 s (statement_timeout) | `503` + `Retry-After: 1` | `{error:{code:"concurrent_request", message:"Another request with this key is in progress"}}` |
| DB error during `begin` (other than timeout) | `500` | `{error:{code:"internal", message:"..."}}` — no message created. |
| DB error during `finalize` | (response unchanged) | Warning logged; message already enqueued; next retry hits stale-lock recovery. |
| Request body > 1 MiB (`MAX_REQUEST_BODY_BYTES`) | `413` | `{error:{code:"payload_too_large"}}` — idempotency check skipped. |
| Response body > 64 KiB (`MAX_RESPONSE_BODY_BYTES`) | (response unchanged) | Warning logged; response **not** cached — next retry treated as fresh. |

**Logging:** every response includes `idempotency_key` (when present) in structured logs alongside the existing `request_id` for Loki correlation.

**Metrics (Prometheus):**
- `chorus_idempotency_outcomes_total{outcome="fresh|replay|hash_mismatch|in_flight|skip|invalid_key"}` (counter)
- `chorus_idempotency_table_rows` (gauge, set by cleanup task each tick)
- `chorus_idempotency_lookup_duration_seconds` (histogram)

## 7. Testing Strategy

### 7.1 Unit tests (`idempotency.rs`)
- `is_valid_key`: empty → false; 255 chars → true; 256 chars → false; ASCII printable + spaces → true; tabs/newlines → false; non-ASCII → false.
- `sha256` stability: `hash(b"")` is the known constant; `hash(b"{\"to\":\"+66\"}")` ≠ `hash(b"{ \"to\":\"+66\" }")` (whitespace matters).

### 7.2 Repository tests (`db::idempotency`, `sqlx::test`)
- First `begin` → `Fresh`, row exists with status='in_progress'.
- Second `begin` (same key + hash, after `complete`) → `Replay` with cached body.
- Second `begin` (same key, different hash) → `HashMismatch`.
- Second `begin` while previous is `in_progress` ≤60 s → `InFlight`.
- Second `begin` while previous is `in_progress` >60 s → `Fresh` (stale recovery).
- `complete` updates status, response_status, response_body atomically.
- `delete_expired(100)` deletes only `expires_at < now()`.
- Concurrent `begin` from two connections (`tokio::join!`) → one gets `Fresh`, other blocks until first completes, then gets `Replay`.
- `ON DELETE CASCADE`: deleting `api_keys` row removes its idempotency rows.

### 7.3 API integration tests (`tests/api_test.rs`)

Fresh / replay:
- POST without header → 202 (regression).
- POST with header (new key) → 202; idempotency_keys row exists status='completed'.
- Repeat with same key+body → 202, identical bytes; messages table has 1 row.

Hash mismatch:
- Same key, different body → 422 idempotency_key_reused; messages still 1 row.

Invalid header:
- Empty / 256 chars / non-ASCII → 400 invalid_idempotency_key.

Cross-route:
- `/v1/sms` (key=abc, body=X) then `/v1/email` (key=abc, body=X) → second call returns 422 (path differs → hash via path differs).

Cross-API-key isolation:
- account A, key K1: key=abc → 202 + M1.
- account A, key K2: key=abc → 202 + M2 (new message).

Batch replay:
- POST `/v1/sms/batch` (key=abc, recipients=[A,B,C]) → 207 with partial result.
- Repeat same key → 207 byte-for-byte identical; messages table unchanged.

OTP integration:
- POST `/v1/otp` (key=abc, phone=+66X) → 202.
- Repeat → 202; only 1 OTP code generated, 1 SMS enqueued.

Error replay:
- POST `/v1/sms` with suppressed recipient → 422 recipient_suppressed.
- Repeat same key → 422 recipient_suppressed (cached error response).

### 7.4 Concurrent / race tests
- Spawn 2 concurrent POSTs with same key+body → one returns 202 with `message_id=M`, the other returns 202 with the same `message_id=M`; messages table has 1 row.
- Manually INSERT row status='in_progress' created_at=now() → POST with same key → 503 concurrent_request after ~5 s; response includes `Retry-After`.

### 7.5 Cleanup task tests
- 5 rows: 3 expired, 2 fresh → `delete_expired(100)` removes 3.
- 100 expired rows → `delete_expired(10)` removes 10; 90 remain.

### 7.6 Smoke test (manual, podman)

Per `feedback_smoke_tests.md`:
```bash
KEY="ch_test_..."
IDEM="smoke-$(date +%s)"

# 1. First send
curl -H "X-API-Key: $KEY" -H "Idempotency-Key: $IDEM" \
  -d '{"to":"+66...","body":"test"}' http://localhost:8080/v1/sms

# 2. Retry (same key, same body) → identical response
curl -H "X-API-Key: $KEY" -H "Idempotency-Key: $IDEM" \
  -d '{"to":"+66...","body":"test"}' http://localhost:8080/v1/sms

# 3. Retry with different body → 422
curl -H "X-API-Key: $KEY" -H "Idempotency-Key: $IDEM" \
  -d '{"to":"+66...","body":"different"}' http://localhost:8080/v1/sms

# 4. Inspect Postgres
psql -c "SELECT idempotency_key, status, response_status, expires_at
         FROM idempotency_keys ORDER BY created_at DESC LIMIT 5;"
```

## 8. Follow-up Criteria (Out of Scope — Trigger Conditions)

| Trigger | Follow-up spec |
|---|---|
| `chorus_idempotency_lookup_duration_seconds{quantile="0.95"} > 0.01` for 7 consecutive days | C1.cache — add Valkey read-through cache layer. |
| `idempotency_keys` rows > 5M (cleanup falling behind) | Add `IDEMPOTENCY_TTL_HOURS` config; consider lowering default to 12 h; raise cleanup tick rate. |
| Enterprise customer requests idempotency on `/v1/billing/*` | C1.billing — chain through Stripe's idempotency. |
| Open-source consistency cleanup | Separate PR: rename "Redis" → "Valkey" in CLAUDE.md (root + chorus), docker-compose.yml, docs. |
| Customer asks for per-recipient idempotency in batch | C1.batch-granular spec. |

## 9. Implementation Order

Each numbered step is one commit (subagent-driven dev):

1. Migration `008_create_idempotency_keys.sql` + `cargo sqlx prepare`.
2. `db/idempotency.rs` — types, trait, Postgres impl + repo tests (7.2).
3. `idempotency.rs` — types, `is_valid_key`, `sha256`, `begin`, `finalize` + unit tests (7.1).
4. `AppState` wiring — `idempotency_repo()` + injection in `AppState::new`.
5. Route refactor `/v1/sms` — accept `Bytes`, integrate begin/finalize + integration tests.
6. Route refactor `/v1/email` + tests.
7. Route refactor `/v1/messages` + tests.
8. Route refactor `/v1/otp` + tests.
9. Route refactor `/v1/sms/batch` and `/v1/email/batch` — batch-level idempotency + tests (7.3 batch replay).
10. Cleanup task + spawn in `app.rs` + tests (7.5).
11. Concurrent / race tests (7.4).
12. Metrics — `chorus_idempotency_outcomes_total`, `_table_rows`, `_lookup_duration_seconds`.
13. Final CI sweep — `cargo test --workspace`, `cargo clippy --workspace -- -D warnings`, `cargo fmt --all --check`, `cargo deny check`.
14. Smoke test on podman (7.6) before opening PR.

A subsequent implementation plan (via `superpowers:writing-plans`) breaks each step into commit-sized tasks with exact code blocks and file paths.
