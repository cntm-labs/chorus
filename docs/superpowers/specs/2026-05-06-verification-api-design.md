# Verification API + Waterfall — Design

**Status:** Approved
**Date:** 2026-05-06
**Author:** chorus team
**Closes:** roadmap item B1 (differentiator tier #1 — "save 60-80% vs Twilio Verify")

---

## 1. Goal

Ship a Verification API that is **better and cheaper than Twilio Verify** by sending the OTP through the cheapest eligible channel automatically (smart routing across email and SMS), surfacing per-call cost in the response, and enforcing rate limits to prevent abuse.

This is the first deliverable in the **B (differentiator)** tier — the hero feature for sales pitch after the production-safety work (A → C1/C2) is complete.

### Headline economics

- chorus email verification: **$0.0001** (100 micro-USD via Resend)
- chorus SMS verification (US): **$0.005** (5,000 micro-USD via Telnyx)
- Twilio Verify SMS US: **$0.05** (50,000 micro-USD)

→ email = **500× cheaper**, SMS = **10× cheaper**, average waterfall (≈80% email + ≈20% SMS) = **60–80% saving**.

## 2. Scope

### In scope (B1)

- New endpoints under `/v1/verifications/*`:
  - `POST /v1/verifications` — create + smart-routed send
  - `POST /v1/verifications/{id}/check` — verify code
  - `POST /v1/verifications/{id}/resend` — new code, optional channel override
  - `POST /v1/verifications/{id}/cancel` — revoke pending verification
  - `GET  /v1/verifications/{id}` — retrieve
  - `GET  /v1/verifications` — list with pagination
- Multi-recipient input: `{phone?, email?, channels?}` (server picks single channel)
- Smart routing: 1 verification = 1 channel attempt; resend is a separate explicit call
- Channels MVP: **email + sms** only
- Storage: Postgres `verifications` table + Valkey for code (TTL 5 min)
- Rate limits: per-recipient (5/hour) + per-account (100/min), fixed in MVP
- Cost reporting in response (`cost_micro` field) + aggregate on the row
- Defaults: 6-digit code, 5 min TTL, 5 check attempts, 3 resend attempts
- Idempotency-Key support on `create` and `resend` (reuses C1)
- Suppression integration (pre-waterfall lookup; bounce → suppression flow unchanged)
- Recipient validation via existing `suppression::normalize`
- Backward-compat: keep `/v1/otp/*` working as legacy (deprecated label only)

### Out of scope (deferred to follow-up specs)

- **B2** TOTP / Authenticator app (RFC 6238, $0/check)
- **B3** WhatsApp channel
- **B4** Voice (TTS) channel
- **B5** Magic link (email passwordless)
- **B6** SDK auto-fill helpers (Android SMS Retriever, iOS verify codes hint)
- **B7** Phone Lookup / fraud detection (carrier, line type, voip)
- Configurable code length / TTL / attempts per account
- Service abstraction (Twilio Service-level config) — chorus stays flat per-account
- Cross-account fraud guard (global counter)
- `attempted_recipients` audit column for "why did fallback happen?" debug
- Per-channel templates beyond `app_name`
- SDK migration helpers / deprecation warnings on `chorus.otp.*`

### Non-goals

- Not an MFA / 2FA platform (no user accounts, no sessions)
- Not a long-term enrollment store (TOTP enrollment lives in B2)
- No silent network auth, push notifications, or identity-document verification

## 3. Schema

New migration `services/chorus-server/src/db/migrations/009_create_verifications.sql`:

```sql
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

### Design notes

| Column | Reason |
|---|---|
| `id` UUID PK | Public ID used in `/check`, `/resend`, `/cancel`, `/retrieve`. |
| `account_id` + `api_key_id` (cascade) | Rotating/deleting an API key clears its history. |
| `channel` (enum) | The channel actually selected by smart routing. |
| `recipient` (canonical) | Normalized form (lowercase email / validated E.164 SMS). Only the channel actually sent to is stored — minimizes PII surface. |
| `status` (4-value enum) | `pending` on create; `approved` after correct check; `canceled` after `/cancel` or check lockout; `expired` after cleanup task or on-demand evaluation. |
| `check_attempts` / `resend_attempts` | Separate counters; resend resets `check_attempts` to 0 (fresh budget for new code). |
| `cost_micro` BIGINT | Aggregate cost across initial send + all resends. Mirrors `messages.cost_microdollars` pattern. |
| `expires_at` partial index | Cleanup task scans only the pending subset — small, fast. |
| **Not stored:** `code` | Code lives in Valkey only — Postgres never stores plaintext code. |
| **Not stored:** both phone+email | Audit trail of "what fallback happened" deferred to follow-up. |

### Valkey keys (TTL = 300s)

```
verify:{verification_id}             → JSON: {code, recipient_hash, check_attempts}
verify:rl:rcpt:{sha256(recipient)}   → ZSET (sliding 1h window)
verify:rl:acct:{account_id}          → ZSET (sliding 1m window)
```

`verify:{id}` is `SET EX 300` on create + resend (overwrite invalidates the previous code). `DEL` on check-success / cancel / lockout.

Rate-limit ZSETs use `ZADD now:now` + `ZREMRANGEBYSCORE` to maintain a sliding window, executed atomically inside a Lua script. The ZSET key carries a TTL of one window (1h or 1m) to prevent orphaned keys.

### Capacity estimate

- Verification row: ~250 bytes overhead.
- 1M verifications/day × 30 days ≈ 7.5 GB; cleanup task expires pending rows aggressively to keep working set small.
- Valkey memory: 1k pending verifications × ~200 bytes ≈ 200 KB — negligible.

## 4. Components

### 4.1 `db::verification` (new file `services/chorus-server/src/db/verification.rs`)

Types in `db/mod.rs`:

```rust
#[derive(Debug, Clone, sqlx::FromRow, Serialize, Deserialize)]
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

pub struct NewVerification {
    pub account_id: Uuid,
    pub api_key_id: Uuid,
    pub channel: String,
    pub recipient: String,
    pub environment: String,
    pub app_name: Option<String>,
    pub initial_cost_micro: i64,
}

#[async_trait]
pub trait VerificationRepository: Send + Sync {
    async fn insert(&self, v: &NewVerification) -> Result<Verification, DbError>;
    async fn find_by_id(&self, id: Uuid, account_id: Uuid) -> Result<Option<Verification>, DbError>;
    async fn list_by_account(
        &self,
        account_id: Uuid,
        pagination: &Pagination,
    ) -> Result<Vec<Verification>, DbError>;

    /// Atomic UPDATE ... SET check_attempts = check_attempts + 1 RETURNING check_attempts.
    /// Returns NotFound if status != 'pending'.
    async fn increment_check_attempts(&self, id: Uuid, account_id: Uuid) -> Result<i32, DbError>;

    async fn mark_approved(&self, id: Uuid, account_id: Uuid) -> Result<(), DbError>;

    /// Returns true if the row was actually canceled (was pending).
    async fn mark_canceled(&self, id: Uuid, account_id: Uuid) -> Result<bool, DbError>;

    /// Atomic resend: increments resend_attempts, adds cost, resets check_attempts.
    /// Errors with NotFound if not pending or if resend cap reached.
    async fn record_resend(
        &self,
        id: Uuid,
        account_id: Uuid,
        additional_cost_micro: i64,
        max_resends: i32,
    ) -> Result<Verification, DbError>;

    /// Background cleanup: bulk-mark expired pending rows. Returns count.
    async fn expire_pending(&self, limit: i64) -> Result<u64, DbError>;
}
```

`record_resend` SQL:
```sql
UPDATE verifications
SET resend_attempts = resend_attempts + 1,
    cost_micro      = cost_micro + $3,
    check_attempts  = 0,
    updated_at      = now()
WHERE id = $1 AND account_id = $2
  AND status = 'pending'
  AND resend_attempts < $4
RETURNING *;
```
Returns 0 rows → `DbError::NotFound` → handler maps to 422 `max_resends_reached` / 410 `<terminal>`.

`expire_pending`:
```sql
UPDATE verifications
SET status = 'expired', updated_at = now()
WHERE id IN (
    SELECT id FROM verifications
    WHERE status = 'pending' AND expires_at < now()
    ORDER BY expires_at
    LIMIT $1
);
```
Uses `verifications_pending_expiry_idx` (partial index → narrow scan).

### 4.2 `verification.rs` (new file `services/chorus-server/src/verification.rs`)

```rust
//! Verification orchestration: smart routing, code lifecycle, rate limiting, pricing.

pub const CODE_LENGTH:           usize = 6;
pub const TTL_SECONDS:           u64   = 300;
pub const MAX_CHECK_ATTEMPTS:    i32   = 5;
pub const MAX_RESEND_ATTEMPTS:   i32   = 3;
pub const RATE_LIMIT_PER_RCPT_HOUR:    u32 = 5;
pub const RATE_LIMIT_PER_ACCT_MINUTE:  u32 = 100;

pub enum ChannelChoice {
    Email { recipient: String, cost_micro: i64 },
    Sms   { recipient: String, cost_micro: i64 },
}

pub enum RoutingError {
    NoRecipient,
    InvalidPhone,
    InvalidEmail,
    NoEligibleChannel,
    RateLimitedRecipient { retry_after_sec: u64 },
    RateLimitedAccount   { retry_after_sec: u64 },
    Db(DbError),
}

/// Smart routing: pick the cheapest eligible channel per the user's `channels`
/// preference order. Returns the canonical recipient and the cost in micro-USD.
pub async fn select_channel(
    state: &Arc<AppState>,
    account_id: Uuid,
    phone: Option<&str>,
    email: Option<&str>,
    channels: &[String],
) -> Result<ChannelChoice, RoutingError>;

/// Pricing lookup. Email is flat; SMS uses E.164 country prefix.
pub fn cost_for(channel: &str, recipient: &str) -> i64;

pub fn generate_code() -> String;
pub async fn store_code(redis: &redis::Client, id: Uuid, recipient: &str, code: &str) -> anyhow::Result<()>;
pub async fn check_code(redis: &redis::Client, id: Uuid, code: &str) -> anyhow::Result<bool>;
pub async fn invalidate_code(redis: &redis::Client, id: Uuid) -> anyhow::Result<()>;

/// Lua-atomic sliding-window check across both rate-limit layers.
pub async fn check_rate_limits(
    redis: &redis::Client,
    account_id: Uuid,
    recipient_hash: &str,
) -> Result<(), RoutingError>;

/// Background loop. Spawn from main.rs alongside idempotency::cleanup_loop.
pub async fn expire_pending_loop(state: Arc<AppState>);
```

The Valkey-side `verify:{id}` payload carries `check_attempts` for fast Lua-side increment, but the **authoritative** counter lives in Postgres (queried + bumped on every `/check` for audit). The Valkey counter is only used inside `check_code` to short-circuit when the code is already gone.

### 4.3 Pricing (`services/chorus-server/src/verification/pricing.rs`)

```rust
pub fn cost_for(channel: &str, recipient: &str) -> i64 {
    match channel {
        "email" => 100,                                            // $0.0001
        "sms"   => sms_cost_for_country(extract_country(recipient)),
        _       => 0,
    }
}

fn sms_cost_for_country(cc: &str) -> i64 {
    match cc {
        "US" | "CA" => 5_000,    // $0.005
        "TH"        => 6_000,
        _           => 8_000,
    }
}

fn extract_country(e164: &str) -> &str {
    // +1.. → US, +66.. → TH, +44.. → UK, etc. Returns "??" on no match.
}
```

Pricing lives in `chorus-server` (not `chorus-core`) for now — keeps the core leaf crate decoupled from rates that change. Move to `chorus-core` only if SDKs need it.

### 4.4 Routes (new file `services/chorus-server/src/routes/verifications.rs`)

Each handler follows the C1 pattern: `Bytes` body + `idempotency::begin/finalize_and_respond` for `create` and `resend`.

```rust
pub async fn create_verification(
    State(state): State<Arc<AppState>>,
    ctx: AccountContext,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    // 1. idempotency::begin
    // 2. parse + validate (phone/email/channels)
    // 3. verification::check_rate_limits
    // 4. verification::select_channel → ChannelChoice
    // 5. verification_repo.insert(channel, recipient, cost_micro)
    // 6. verification::store_code(redis, id, recipient, code)
    // 7. message_repo.insert + queue::enqueue::notify
    // 8. finalize_and_respond → 201 + Verification body
}

pub async fn check_verification(...);
pub async fn resend_verification(...);
pub async fn cancel_verification(...);
pub async fn get_verification(...);
pub async fn list_verifications(...);
```

Wired in `app.rs::create_router_with_metrics`:
```rust
.route("/v1/verifications", post(routes::verifications::create_verification)
                                  .get(routes::verifications::list_verifications))
.route("/v1/verifications/{id}",          get(routes::verifications::get_verification))
.route("/v1/verifications/{id}/check",    post(routes::verifications::check_verification))
.route("/v1/verifications/{id}/resend",   post(routes::verifications::resend_verification))
.route("/v1/verifications/{id}/cancel",   post(routes::verifications::cancel_verification))
```

The legacy `/v1/otp/send` and `/v1/otp/verify` routes stay untouched — they keep their C1 idempotency wiring.

### 4.5 Background cleanup

```rust
pub async fn expire_pending_loop(state: Arc<AppState>) {
    let mut tick = tokio::time::interval(Duration::from_secs(60));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tick.tick().await;
        match state.verification_repo().expire_pending(1_000).await {
            Ok(n) if n > 0 => tracing::info!(expired = n, "verifications expired"),
            Ok(_) => {}
            Err(e) => tracing::warn!(error = %e, "verification expire failed"),
        }
    }
}
```

Spawned in `main.rs`:
```rust
tokio::spawn(chorus_server::idempotency::cleanup_loop(Arc::clone(&state)));
tokio::spawn(chorus_server::verification::expire_pending_loop(Arc::clone(&state)));
```

### 4.6 `AppState` wiring

Field `verification_repo: Arc<dyn VerificationRepository>` + accessor + injection in `AppState::new` and the testing-only `with_repos`.

## 5. Data Flow

### 5.1 Create — happy path (email available)

```
POST /v1/verifications
Headers: Idempotency-Key: req-abc
Body:    {"phone":"+66812345678","email":"alice@example.com","app_name":"Acme"}

→ idempotency::begin → Fresh
→ validate (suppression::normalize phone & email; need at least one)
→ verification::check_rate_limits(account, recipient_hash) → ok
→ verification::select_channel(phone, email, ["email","sms"])
    → ChannelChoice::Email { recipient: "alice@example.com", cost_micro: 100 }
→ verification_repo.insert(channel="email", recipient="alice@…",
                           status="pending", cost_micro=100,
                           expires_at=now()+5min)
→ verification::store_code(redis, id, "alice@…", "483921")  // SET verify:{id} EX 300
→ message_repo.insert + queue::enqueue::notify
→ idempotency::finalize_and_respond → 201
   {
     "id": "<uuid>", "status": "pending", "channel": "email",
     "recipient": "alice@example.com",
     "cost_micro": 100, "cost_currency": "USD",
     "expires_at": "...", "check_attempts": 0, "resend_attempts": 0
   }
```

### 5.2 Create — fallback to SMS (email suppressed)

```
select_channel: channels[0]="email" → suppressed → channels[1]="sms"
                "+66812345678" valid + not suppressed
→ ChannelChoice::Sms { cost_micro: 6000 }   (TH, $0.006)
→ Response: { "channel": "sms", "cost_micro": 6000, ... }
```

### 5.3 Create — no eligible channel

| Input | Result |
|---|---|
| neither phone nor email | 400 `no_recipient` |
| phone fails E.164 / email fails format | 400 `invalid_phone` / 400 `invalid_email` |
| both suppressed (or both invalid after the above) | 422 `no_eligible_channel` |

### 5.4 Check

```
POST /v1/verifications/{id}/check
Body: {"code":"483921"}

→ load verification by (id, account_id) → 404 if absent
→ guard:
   approved → 410 already_verified
   canceled → 410 canceled
   expired  → 410 expired
   pending  → continue
→ verification_repo.increment_check_attempts(id, account)
   if new count > MAX_CHECK_ATTEMPTS:
       DEL Valkey verify:{id}
       mark status='canceled' (lockout)
       return 410 max_attempts_exceeded
→ verification::check_code(redis, id, "483921")
   match    → DEL verify:{id}; mark_approved → 200 { status: "approved", ... }
   mismatch → 422 incorrect_code (with attempts_remaining)
   gone     → 410 expired
```

### 5.5 Resend

```
POST /v1/verifications/{id}/resend
Headers: Idempotency-Key: req-resend-1
Body:    {"channel":"sms"}   // optional override

→ idempotency::begin
→ load verification → guard pending
→ verification::check_rate_limits — **per-account only** (see note below)
→ pick channel: body.channel if eligible, else verification.channel
→ generate new code (different from previous)
→ store_code(redis, id, recipient, new_code)   // OVERWRITE — old code invalidated
→ verification_repo.record_resend(id, account, cost_for(new_channel, recipient), 3)
   → 0 rows → 422 max_resends_reached
   → 1 row  → returns updated verification
→ message_repo.insert + queue::enqueue::notify
→ Response: { channel: <new>, cost_micro: <aggregate>, resend_attempts: 1, ... }
```

The `expires_at` is **not** extended — the original 5-minute window remains, preventing attackers from indefinitely prolonging a verification.

**Rate-limit policy on resend:**
- Per-recipient limit (5/hour): **not** re-checked. Bounded already by `MAX_RESEND_ATTEMPTS = 3` per verification, and the limit was applied when the verification was created.
- Per-account limit (100/min): **re-checked**. A compromised account could otherwise burst resends across many existing verifications without throttling.

### 5.6 Cancel

```
POST /v1/verifications/{id}/cancel

→ verification_repo.mark_canceled(id, account)
   → 0 rows → 410 already_terminal
   → 1 row  → DEL verify:{id} from Valkey
→ Response: 200 { status: "canceled", ... }
```

### 5.7 Background expiry

Every 60 seconds the cleanup loop sweeps up to 1,000 expired pending rows, transitioning them to `expired`. Uses the partial index `verifications_pending_expiry_idx` to keep the scan small.

## 6. Error Handling

| Endpoint | Condition | HTTP | `error.code` |
|---|---|---|---|
| Create | no phone/email | 400 | `no_recipient` |
| Create | bad E.164 | 400 | `invalid_phone` |
| Create | bad email format | 400 | `invalid_email` |
| Create | both channels suppressed/missing | 422 | `no_eligible_channel` |
| Create | per-recipient rate limit | 429 + `Retry-After` | `rate_limited` |
| Create | per-account rate limit | 429 + `Retry-After` | `rate_limited` |
| Check | unknown id | 404 | `not_found` |
| Check | terminal status | 410 | `already_verified` / `canceled` / `expired` |
| Check | wrong code | 422 | `incorrect_code` (with `attempts_remaining`) |
| Check | check_attempts > MAX | 410 | `max_attempts_exceeded` (and lockout) |
| Check | code missing in Valkey | 410 | `expired` |
| Resend | resend_attempts ≥ MAX | 422 | `max_resends_reached` |
| Resend | terminal status | 410 | `<status>` |
| Resend | overridden channel ineligible | 422 | `no_eligible_channel` |
| Cancel | already terminal | 410 | `already_terminal` |
| Get/List | unknown id / cross-account | 404 | `not_found` |
| Any | DB error | 500 | `internal` |
| Create / Resend | Idempotency-Key reused with different body | 422 | `idempotency_key_reused` (from C1) |

### Logging

Every response includes:
- `verification_id` (when known)
- `account_id`, `api_key_id`
- `channel` (selected)
- `recipient_hash` (sha256 — never log plaintext PII)
- `request_id` (from existing middleware)
- `cost_micro` (on create / resend)

### Metrics (Prometheus)

```
chorus_verifications_total{channel, outcome}
  outcome ∈ {created, approved, incorrect_code, expired, canceled,
             rate_limited, max_resends, max_attempts}

chorus_verifications_routing_total{chosen_channel, fallback_reason}
  fallback_reason ∈ {primary_chosen, email_suppressed, email_missing,
                     sms_suppressed, sms_invalid}

chorus_verifications_create_duration_seconds  (histogram)
chorus_verifications_check_duration_seconds   (histogram)
chorus_verifications_pending_total            (gauge, set by cleanup loop)
chorus_verifications_cost_micro_total{channel} (counter)
```

## 7. Testing Strategy

### 7.1 Unit tests (`verification.rs`)

- `generate_code` returns 6 digits, 100 calls show no collision
- `cost_for("email", _)` = 100
- `cost_for("sms", "+14155552671")` = 5,000 (US)
- `cost_for("sms", "+66812345678")` = 6,000 (TH)
- `cost_for("sms", "+33123…")` = 8,000 (fallback)
- `extract_country` for `+1`, `+66`, `+44` and unknown prefixes

### 7.2 Repository tests (`sqlx::test`, ignored by default)

- `insert_creates_pending_row_with_expiry`
- `find_by_id_scopes_to_account`
- `list_by_account_orders_desc + pagination`
- `increment_check_attempts_atomic`
- `mark_approved_only_if_pending`
- `mark_canceled_only_if_pending`
- `record_resend_increments_and_adds_cost`
- `record_resend_returns_notfound_when_max_reached`
- `record_resend_returns_notfound_when_status_terminal`
- `expire_pending_only_picks_expired_pending`
- `cascade_delete_on_api_key_removal`

### 7.3 Routing logic tests

With `MockSuppressionRepo` + `MockRateLimit`, no DB / Valkey required:

- `email_first_picks_email_when_eligible`
- `falls_back_to_sms_when_email_suppressed`
- `falls_back_to_sms_when_email_missing`
- `channels_array_respects_user_order`
- `returns_no_eligible_when_both_suppressed`
- `returns_invalid_phone_when_e164_fails`
- `returns_invalid_email_when_format_fails`
- `rate_limited_recipient_returns_retry_after`
- `rate_limited_account_returns_retry_after`
- `recipient_rate_limit_isolated_from_account_rate_limit`

### 7.4 API integration tests (`tests/api_test.rs`)

`MockVerificationRepo` (in-memory HashMap) + `MockRateLimitRepo`:

- `create_verification_with_email_returns_201_with_email_channel`
- `create_verification_with_phone_only_returns_201_with_sms_channel`
- `create_verification_idempotency_replay_byte_for_byte` (C1 pattern)
- `create_verification_falls_back_to_sms_when_email_suppressed`
- `create_verification_returns_422_when_no_eligible_channel`
- `create_verification_returns_400_when_no_recipient`
- `create_verification_returns_400_when_invalid_phone`
- `create_verification_returns_400_when_invalid_email`
- `check_with_correct_code_returns_approved`
- `check_with_wrong_code_returns_422_incorrect_code_with_attempts_remaining`
- `check_after_max_attempts_returns_410_max_attempts_exceeded`
- `check_canceled_verification_returns_410`
- `check_expired_verification_returns_410`
- `resend_with_same_channel_uses_new_code` (old code → 422)
- `resend_with_channel_override_changes_to_sms`
- `resend_after_max_returns_422_max_resends`
- `resend_idempotency_replay`
- `resend_resets_check_attempts_to_zero`
- `cancel_pending_returns_canceled_status`
- `cancel_already_canceled_returns_410`
- `cancel_invalidates_redis_code_so_check_returns_410`
- `get_returns_full_verification_object`
- `get_other_account_verification_returns_404`
- `list_paginates + filter`
- `per_recipient_rate_limit_returns_429_with_retry_after`
- `per_account_rate_limit_returns_429`
- `rate_limit_isolated_per_recipient`
- `create_response_includes_cost_micro` (email=100, US SMS=5000)
- `resend_aggregates_cost_in_response`
- `get_returns_aggregate_cost`
- `legacy_v1_otp_send_still_works` (regression — C1 idempotency intact)
- `legacy_v1_otp_verify_still_works`

### 7.5 Smoke test on podman

Following the C1 pattern (boot fresh `postgres:16-alpine` and `valkey/valkey:8-alpine`, run server natively, seed account/api_key/suppression by SQL, then curl):

1. Create with email → expect `channel:"email"`, `cost_micro:100`
2. Replay same `Idempotency-Key` → byte-for-byte match
3. Suppress email → repeat create → expect `channel:"sms"`, `cost_micro:6000`
4. Suppress sms too → 422 `no_eligible_channel`
5. Loop 6 creates to same recipient → 6th returns 429 with `Retry-After`
6. Read code from Valkey → `/check` → `status:"approved"`
7. `psql` inspection of the `verifications` table
8. `curl /metrics | grep chorus_verifications`

## 8. Follow-up Criteria (Out of Scope — Trigger Conditions)

| Trigger | Follow-up |
|---|---|
| Customers ask for non-numeric / configurable code length | Per-account `verification_settings` (B1.config) |
| Customers ask for TOTP support | **B2** — RFC 6238 enrollment + verify, $0/check |
| Customers in WhatsApp-heavy markets (BR, IN, ID) | **B3** — WhatsApp Business Cloud channel |
| Accessibility need (visually impaired users) | **B4** — voice/TTS channel via Telnyx Voice |
| Passwordless email-only flows | **B5** — magic link instead of code |
| SMS-retriever auto-fill complaints from mobile teams | **B6** — SDK helpers per platform |
| Toll-fraud / VoIP abuse detected via metrics | **B7** — phone Lookup integration |
| `chorus_verifications_routing_total` shows >5% fallback to SMS for email-eligible accounts | Investigate suppression false-positives or per-recipient cooldowns |
| `chorus_verifications_pending_total` grows unboundedly | Increase cleanup batch size or tick rate |

## 9. Implementation Order

Each numbered step is one commit (subagent-driven dev pattern):

1. Migration `009_create_verifications.sql` (table + indexes + partial index).
2. Repository trait + types in `db/mod.rs`; empty `db/verification.rs`.
3. `PgVerificationRepository` impl + `sqlx::test` cases (ignored by default).
4. `verification.rs` constants, types, `generate_code`, pricing helpers + unit tests.
5. `verification.rs` Valkey helpers (`store_code`, `check_code`, `invalidate_code`).
6. Rate limiting (Lua script + `check_rate_limits`) + unit tests with mock Valkey.
7. `select_channel` smart routing + unit tests with mock suppression + rate-limit.
8. `AppState` wiring (`verification_repo` field, accessor, `with_repos` arg).
9. Routes `routes/verifications.rs` — create + check + cancel + get + list.
10. Routes — resend (separate task, multi-step logic).
11. API integration tests — happy path + validation + routing + cost.
12. API integration tests — rate limits + idempotency replay + backward compat.
13. Background `expire_pending_loop` + spawn in `main.rs`.
14. Prometheus metrics per §6.
15. Final CI sweep — `cargo fmt --all`, `cargo clippy --workspace -- -D warnings`, `cargo test --workspace`, `cargo deny check`.
16. Smoke test on podman (§7.5) before opening the PR.

A subsequent implementation plan (via `superpowers:writing-plans`) breaks each step into commit-sized tasks with exact code blocks and file paths.
