# Magic Link (Passwordless) Verification — Design

**Status:** Approved
**Date:** 2026-05-29
**Author:** chorus team
**Closes:** roadmap item B5 (differentiator tier #3 — "passwordless verification like Slack/Notion")

---

## 1. Goal

Magic-link (passwordless) verification at `/v1/verifications/*` channel — extend B1 architecture with a stateful random token + redirect callback pattern (Auth0/Cognito/Stripe style). Customer's backend confirms approval via existing `GET /v1/verifications/{id}` server-side.

**Hero pitch:** "passwordless verification like Slack/Notion — chorus = **$0.0001/link** vs Auth0 = **$0.045/MAU** = 450× cheaper, no per-link cost."

Third deliverable of the B (differentiator) tier after B1 (Verification API + Waterfall) and B2 (TOTP).

### Economics positioning

| Operation | chorus | Auth0 | Twilio Verify |
|---|---|---|---|
| Magic link send | **$0.0001** | $0.045/MAU | (no equivalent) |
| Magic link verify (server poll) | **$0** | included | — |

→ Customer at 10k magic links/month: chorus = $1/mo, Auth0 = $450/mo (1 MAU per link).

## 2. Scope

### In scope (B5)

- **Migration 011** — extends B1's `verifications` table:
  - relax `channel` CHECK to `IN ('sms','email','magic_link')`
  - ADD `magic_link_token_hash BYTEA` nullable (SHA-256 of random token)
  - ADD `magic_link_redirect_url TEXT` nullable (per-request override)
  - ADD partial index on `magic_link_token_hash WHERE NOT NULL`
  - ALTER `accounts` ADD `magic_link_allowed_redirects TEXT[] NOT NULL DEFAULT '{}'`
- **API surface (extends B1):**
  - `POST /v1/verifications {email, channels:["magic_link"], redirect_url?, app_name?}` — existing endpoint, new channel
  - `GET /v1/verifications/callback?token=<base64url>` — **NEW, no auth** (public callback)
  - `GET /v1/verifications/{id}` — existing, used by customer backend to server-side verify
  - `POST /v1/verifications/{id}/resend` — existing, generates new token + invalidates old
  - `DELETE /v1/verifications/{id}` — existing, cancels pending magic link (clears `magic_link_token_hash`)
- **Token semantics:**
  - 32 random bytes (CSPRNG via `OsRng`) → `base64url` encode (~43 chars) for URL
  - SHA-256 hash stored in DB (`magic_link_token_hash`)
  - One-time use: atomic `UPDATE ... SET status='approved', magic_link_token_hash=NULL ... WHERE token_hash=$1 AND status='pending' AND expires_at>now() RETURNING ...`
  - 1h TTL (vs B1's 5min for OTP)
- **Redirect URL validation:**
  - Per-account `magic_link_allowed_redirects` array
  - https-only (except `localhost` / `127.0.0.1` for dev)
  - Prefix match: scheme + host + port exact, path starts-with
  - Per-request `redirect_url` validated against whitelist → 400 `redirect_not_whitelisted` if no match
  - No `redirect_url` in request → fallback chain: `magic_link_allowed_redirects[0]` → chorus landing page
- **Server-side verify pattern (Auth0/Cognito style):**
  - Callback redirects to `{redirect_url}?verification_id={uuid}` (or `&verification_id=...` if base URL already has a query)
  - Customer's backend calls `GET /v1/verifications/{id}` (authenticated with API key) to confirm `status=="approved"` + recipient
  - No webhook in MVP — customer polls; webhook deferred to B5.2
- **Email content:**
  - chorus-generated default template (subject + plain-text body)
  - Per-request `app_name` substitution
  - "If you didn't request this, ignore" anti-phishing line
  - Reuses B1's email pipeline (`message_repo.insert` → `queue::notify`)
- **Idempotency:** inherited from B1 — same Idempotency-Key + same body returns same verification (= same token = same URL)
- **Rate limiting:** inherited from B1 — per-recipient 5/hour + per-account 100/min (Lua sliding window)
- **Resend:** inherited — new token, invalidates old, does NOT extend TTL, cap 3
- **Cost:** `cost_micro = 100` (email channel pricing)
- **Prometheus metrics:** reuse B1's `chorus_verifications_total{channel="magic_link",outcome=...}` + new `chorus_verifications_magic_link_callbacks_total{outcome=...}`

### Out of scope (deferred to follow-up specs)

- **B5.1** Per-account email template (HTML + plaintext, admin endpoint, minijinja rendering)
- **B5.2** Webhook on click (`magic_link.approved` event POST to customer URL)
- **B5.3** Cross-channel waterfall with `magic_link` (`channels:["magic_link","sms"]`)
- **B5.4** Admin endpoint for `magic_link_allowed_redirects` (managed via direct SQL in MVP)
- **B5.5** Custom landing-page branding (chorus default landing only)
- **B5.6** Click tracking analytics (open rate, time-to-click)

### Non-goals

- Not an identity provider — chorus doesn't manage sessions; customer backend creates sessions after `GET /v1/verifications/{id}` confirms approval.
- Not "Sign in with chorus" — no OAuth flow, no user attribute store.
- No SMS magic link (= SMS containing a URL) — email only in MVP.
- No email open tracking / pixel — privacy-respecting.

## 3. Schema

New migration `services/chorus-server/src/db/migrations/011_add_magic_link.sql`:

```sql
-- 1. Relax channel CHECK constraint to allow 'magic_link'
ALTER TABLE verifications
    DROP CONSTRAINT verifications_channel_check,
    ADD CONSTRAINT verifications_channel_check
        CHECK (channel IN ('sms', 'email', 'magic_link'));

-- 2. Magic-link-specific columns (nullable; only populated when channel='magic_link')
ALTER TABLE verifications
    ADD COLUMN magic_link_token_hash BYTEA,
    ADD COLUMN magic_link_redirect_url TEXT;

-- 3. Partial index for fast token lookup on callback
CREATE INDEX verifications_magic_link_token_idx
    ON verifications (magic_link_token_hash)
    WHERE magic_link_token_hash IS NOT NULL;

-- 4. Account-level redirect whitelist
ALTER TABLE accounts
    ADD COLUMN magic_link_allowed_redirects TEXT[] NOT NULL DEFAULT '{}';
```

### Design notes

| Change | Reason |
|---|---|
| Relax `channel` CHECK to add `magic_link` | enables single `verifications` table to host the channel without a separate `magic_links` table — matches Q1 decision A (hybrid) |
| `magic_link_token_hash BYTEA` nullable | only set for magic_link rows; remains NULL for sms/email; SHA-256(token) = 32 bytes; NULL after callback consumes it (atomic one-use) |
| `magic_link_redirect_url TEXT` nullable | per-request override; falls back to account default at callback time if NULL; stored so resend keeps same destination |
| Partial index `WHERE magic_link_token_hash IS NOT NULL` | callback lookup must be fast; partial index keeps the scan tiny (only pending magic-link rows) |
| `accounts.magic_link_allowed_redirects TEXT[]` default `'{}'` | empty array = no magic-link redirects configured → customer must opt in by setting at least one URL (defense-in-depth: prevents accidental open-redirect on fresh accounts) |
| Reuse `expires_at` | existing column; magic_link path sets `now() + interval '1 hour'` instead of `+5 minutes` — no schema change needed |
| Reuse `status` enum | `pending → approved | expired | canceled` — same lifecycle as B1; no new states |
| No new table | considered + rejected (Q1 decision A) |
| `cost_micro` BIGINT | filled with `100` for magic_link (email pricing) — same column as B1 |
| No `token_used_at` | when token consumed, hash → NULL is the "used" marker; `updated_at` records when |

### Token storage

- Plaintext: 32 random bytes from `OsRng` (256 bits)
- URL encoding: `base64::URL_SAFE_NO_PAD` → ~43 chars (vs hex = 64)
- DB: `Sha256::digest(plaintext_bytes)` → 32 bytes BYTEA, irreversible
- Lookup: `Sha256::digest(token_param.as_bytes())` → query `WHERE magic_link_token_hash = $1`

### Capacity estimate

- magic_link row adds ~50-100 bytes (token_hash 32 + redirect_url ~40) on top of B1's ~250 bytes/row → ~350 bytes/row
- 1M magic links/day × 1h TTL → ~42k concurrent pending rows; partial index scope ≤ that
- `accounts.magic_link_allowed_redirects` — typical 2-5 URLs/account → ~100 bytes/account; negligible

### Valkey

**No new Valkey keys.** Rate-limit ZSETs from B1 apply unchanged. Callback is server-side DB-only; no Valkey involvement.

## 4. Components

### 4.1 `verification.rs` — magic_link extensions

```rust
pub const MAGIC_LINK_TTL_SECS: u64 = 3600;    // 1 hour (vs B1's 300 for OTP)
pub const MAGIC_LINK_TOKEN_BYTES: usize = 32; // 256-bit random
pub const MAGIC_LINK_COST_MICRO: i64 = 100;   // = email channel cost

/// Extend ChannelChoice (existing enum) with MagicLink variant.
pub enum ChannelChoice {
    Email { recipient: String, cost_micro: i64 },
    Sms { recipient: String, cost_micro: i64 },
    MagicLink {                                       // NEW
        recipient: String,
        cost_micro: i64,
        redirect_url: Option<String>,                 // validated against whitelist
    },
}

/// Generate a 32-byte random token + return (plaintext, sha256_hash).
pub fn generate_magic_link_token() -> (String, Vec<u8>);

/// Validate a redirect URL against the account's whitelist.
/// https-only (except localhost/127.0.0.1 for dev); prefix match.
pub fn validate_redirect_url(allowed: &[String], req_url: &str) -> Result<(), RoutingError>;

/// Build the URL embedded in the email body.
pub fn build_magic_link_url(public_base_url: &str, token_plaintext: &str) -> String;

/// Append verification_id query param to a redirect URL (handles existing query).
pub fn append_verification_id(base: &str, id: Uuid) -> String;
```

`RoutingError` (existing enum) adds two variants:
- `InvalidRedirectUrl` → 400 `invalid_redirect_url`
- `RedirectNotWhitelisted` → 400 `redirect_not_whitelisted`

### 4.2 `db::verification` — extend repo trait

`NewVerification` (existing struct) adds two nullable fields:

```rust
pub struct NewVerification {
    // ... existing fields ...
    pub magic_link_token_hash: Option<Vec<u8>>,
    pub magic_link_redirect_url: Option<String>,
}
```

New trait method:

```rust
#[derive(Debug, Clone)]
pub struct MagicLinkConsumeResult {
    pub verification_id: Uuid,
    pub account_id: Uuid,
    pub redirect_url: Option<String>,
}

#[async_trait]
pub trait VerificationRepository: Send + Sync {
    // ... existing methods ...

    /// Atomic callback consume.
    /// Returns None if no match (expired, used, canceled, unknown).
    async fn consume_magic_link_token(
        &self,
        token_hash: &[u8],
    ) -> Result<Option<MagicLinkConsumeResult>, DbError>;
}
```

Postgres impl uses `UPDATE ... RETURNING` for atomicity:

```sql
UPDATE verifications
SET status='approved',
    magic_link_token_hash=NULL,
    updated_at=now()
WHERE magic_link_token_hash = $1
  AND status='pending'
  AND expires_at > now()
RETURNING id, account_id, magic_link_redirect_url;
```

Also extends `record_resend` to overwrite `magic_link_token_hash` atomically when the existing row has one.

### 4.3 `db::account` — whitelist accessor

```rust
#[async_trait]
pub trait AccountRepository: Send + Sync {
    // ... existing methods ...

    async fn magic_link_allowed_redirects(
        &self,
        account_id: Uuid,
    ) -> Result<Vec<String>, DbError>;
}
```

### 4.4 Routes — extend `routes/verifications.rs`

**A. Extend `create_verification_inner`** — add a branch in the channel-handling logic:

```rust
match choice {
    ChannelChoice::MagicLink { recipient, cost_micro, redirect_url } => {
        // 1. Validate redirect_url against account whitelist (or fall back later)
        if let Some(req_url) = &redirect_url {
            let allowed = state.account_repo()
                .magic_link_allowed_redirects(ctx.account_id).await?;
            verification::validate_redirect_url(&allowed, req_url)?;
        }
        // 2. Generate token + hash
        let (plaintext, hash) = verification::generate_magic_link_token();
        // 3. INSERT with hash + redirect_url
        // 4. Build magic_link_url from config.public_base_url + plaintext
        // 5. Build email subject + body (different from OTP template)
        // 6. enqueue_verification_send(... &magic_link_url)
    }
    ChannelChoice::Email { .. } | ChannelChoice::Sms { .. } => { /* existing */ }
}
```

**B. NEW handler `callback_verification`** (no auth):

```rust
const CALLBACK_PATH: &str = "/v1/verifications/callback";

#[derive(Deserialize)]
pub struct CallbackParams {
    pub token: String,
}

/// GET /v1/verifications/callback?token=<base64url>
/// PUBLIC endpoint — no auth; user clicks email link.
pub async fn callback_verification(
    State(state): State<Arc<AppState>>,
    Query(params): Query<CallbackParams>,
) -> Response;
```

Returns:
- `302 Found` with `Location: {redirect_url}?verification_id={uuid}` on success
- `410 Gone` + HTML landing on consumed/expired/canceled/unknown token
- `400 Bad Request` + HTML landing on missing/garbled token query param
- `500` + HTML landing on DB error

Helper `chorus_landing_page(status, message)` renders a small HTML page with `Content-Type: text/html`.

**C. Route registration in `app.rs`**

```rust
.route("/v1/verifications/callback", get(routes::verifications::callback_verification))
```

The callback handler does NOT destructure `AccountContext`, so the extractor-level auth is naturally skipped — matches chorus's existing pattern for `/internal/*` and `/health` routes.

### 4.5 `enqueue_verification_send` — extend signature

Add an optional `magic_link_url` parameter; when present, build a different email body using:

```rust
const MAGIC_LINK_SUBJECT_TEMPLATE: &str = "Sign in to {app_name}";
const MAGIC_LINK_BODY_TEMPLATE: &str = "\
Click the link below to sign in to {app_name}:

{magic_link_url}

This link expires in 1 hour and can only be used once.
If you didn't request this, ignore this email.
";
```

Substitution is a literal `replace("{app_name}", app_name).replace("{magic_link_url}", url)` — no minijinja in MVP.

### 4.6 `Config::public_base_url`

```rust
pub struct Config {
    // ... existing ...
    pub public_base_url: String,  // e.g. "https://api.chorus.example.com"
}
```

Loaded from env `CHORUS_PUBLIC_BASE_URL`; default falls back to `http://{host}:{port}` for dev. Required for `build_magic_link_url`. README documents.

### 4.7 AppState

No new field. `account_repo()`, `verification_repo()`, `config()` already exist. `AccountRepository` trait extension + `MockAccountRepo` update in tests.

### 4.8 New deps

```toml
url = { workspace = true }   # already in workspace; for URL parsing in validate_redirect_url
```

`url` is in the workspace.

## 5. Data Flow

### 5.1 Create magic-link verification

```
client →
  POST /v1/verifications
  Headers: Authorization: Bearer ch_…
           Idempotency-Key: req-abc          (optional)
  Body:    {"email":"alice@app.com", "channels":["magic_link"],
            "redirect_url":"https://app.com/welcome?signup=true",
            "app_name":"Acme"}

→ AccountContext (auth)
→ idempotency::begin → Fresh
→ verification::check_rate_limits → ok
→ verification::select_channel → ChannelChoice::MagicLink {
     recipient:"alice@app.com", cost_micro:100,
     redirect_url:Some("https://app.com/welcome?signup=true")
   }
→ verification::validate_redirect_url(account.whitelist, ...) → Ok
→ (plaintext, hash) = generate_magic_link_token()
→ verification_repo.insert(NewVerification{
     channel:"magic_link", recipient, status:"pending",
     expires_at: now()+1h, cost_micro:100,
     magic_link_token_hash: Some(hash),
     magic_link_redirect_url: Some(req.redirect_url),
   })
→ magic_link_url = build_magic_link_url(config.public_base_url, &plaintext)
→ (subject, body) = build_email_for_magic_link("Acme", &magic_link_url)
→ message_repo.insert(NewMessage{channel:"email", recipient, subject, body, ...})
→ queue::enqueue::notify(...)
→ idempotency::finalize_and_respond → 201:
  {"id":"...", "channel":"magic_link", "recipient":"alice@app.com",
   "status":"pending", "cost_micro":100, "cost_currency":"USD",
   "expires_at":"...+1h", "check_attempts":0, "resend_attempts":0,
   "magic_link_redirect_url":"https://app.com/welcome?signup=true"}
```

The `token_hash` is **not** in the response. The plaintext token appears only in the email body.

### 5.2 User clicks the email link

```
user → email client → GET https://api.chorus.example.com/v1/verifications/callback?token=<plaintext>

→ (no auth)
→ token_hash = SHA-256(plaintext.as_bytes())
→ verification_repo.consume_magic_link_token(&token_hash):
    UPDATE verifications
    SET status='approved', magic_link_token_hash=NULL, updated_at=now()
    WHERE magic_link_token_hash=$1 AND status='pending' AND expires_at>now()
    RETURNING id, account_id, magic_link_redirect_url;
→ Some(MagicLinkConsumeResult{verification_id, account_id, redirect_url:Some(...)})

→ resolve final redirect:
    request override present → use it
    else account default → account_repo.magic_link_allowed_redirects[0]
    else → chorus landing page 200 HTML

→ append verification_id:
    "https://app.com/welcome?signup=true&verification_id=<uuid>"

→ 302 Found
   Location: https://app.com/welcome?signup=true&verification_id=<uuid>

→ user's browser follows → lands on customer's page
```

### 5.3 Customer backend confirms (server-side verify)

```
user's browser → customer's frontend → reads verification_id from query
              → POST customer_backend/auth/magic-link-callback {verification_id}

customer_backend →
   GET https://api.chorus.example.com/v1/verifications/<uuid>
   Authorization: Bearer ch_live_…

chorus →
   200 OK {"id":"...", "channel":"magic_link", "recipient":"alice@app.com",
           "status":"approved", "updated_at":"...", "cost_micro":100, ...}

customer_backend → verify status=='approved' AND recipient == expected email
                → lookup user in own DB → create session → set cookie
                → return to frontend; user is logged in
```

Customer's backend MUST verify both `status == "approved"` AND `recipient == <expected user email>` to prevent token replay across sessions.

### 5.4 Callback failure modes

| User action | DB state | chorus response |
|---|---|---|
| Click valid pending link | `pending` + token matches + not expired | UPDATE→approved; 302 redirect |
| Click already-used link | `approved` + hash=NULL | 0 rows → 410 + HTML "This link has expired or has already been used." |
| Click expired link | `pending` + `expires_at < now()` | 0 rows → 410 + same landing |
| Click canceled link | `canceled` + hash=NULL | 0 rows → 410 + same landing |
| Click garbled token | no hash match | 0 rows → 410 + same landing |
| Missing `?token=` | — | 400 + HTML "Invalid link" |
| No per-request URL, account default present | succeeded | 302 to `account.magic_link_allowed_redirects[0]?verification_id=<uuid>` |
| No per-request URL, no account default | succeeded | 200 + HTML chorus landing "Verified — you can close this tab." |

### 5.5 Cancel + resend

**Cancel** (`DELETE /v1/verifications/{id}`):
- `UPDATE ... SET status='canceled', magic_link_token_hash=NULL ...`
- Clearing `magic_link_token_hash` is added behavior for the magic_link path; for sms/email it's a no-op (already NULL).
- Subsequent clicks on the link → 410.

**Resend** (`POST /v1/verifications/{id}/resend`):
- Generate new token + hash → atomically `UPDATE verifications SET magic_link_token_hash=new_hash, resend_attempts=resend_attempts+1, ...` (capped at 3)
- Old token invalidated by the UPDATE (hash overwritten)
- New email sent with new URL
- `expires_at` **NOT** extended (preserve original 1h window — security: prevent resend-based TTL extension)
- Max 3 resends per verification (B1 cap)

## 6. Error Handling + Metrics

### 6.1 HTTP error matrix

| Endpoint | Condition | HTTP | `error.code` |
|---|---|---|---|
| `POST /v1/verifications` | email missing when channels includes magic_link | 400 | `no_recipient` (existing) |
| `POST /v1/verifications` | invalid email format | 400 | `invalid_email` (existing) |
| `POST /v1/verifications` | redirect_url not parseable / not https | 400 | `invalid_redirect_url` (NEW) |
| `POST /v1/verifications` | redirect_url valid but not in account whitelist | 400 | `redirect_not_whitelisted` (NEW) |
| `POST /v1/verifications` | email suppressed | 422 | `no_eligible_channel` (existing) |
| `POST /v1/verifications` | rate-limited | 429 | `rate_limited` (existing) |
| `GET /v1/verifications/callback` | token missing from query | 400 + HTML | landing "Invalid link" |
| `GET /v1/verifications/callback` | token doesn't match any pending row | 410 + HTML | landing "This link has expired or has already been used." |
| `GET /v1/verifications/callback` | DB error during UPDATE | 500 + HTML | landing "Something went wrong. Please try again." |
| `POST /v1/verifications/{id}/resend` | 3 resends used | 422 | `max_resends_reached` (existing) |
| `DELETE /v1/verifications/{id}` | already canceled/approved | 410 | `already_terminal` (existing) |
| Any | Idempotency-Key reused with different body | 422 | `idempotency_key_reused` (C1) |

### 6.2 Open-redirect defense layers (defense-in-depth)

1. **Whitelist validation at create time** — rejects `https://evil.com/...`
2. **https-only** (except `localhost` / `127.0.0.1` for dev) — prevents downgrade
3. **scheme + host + port exact** — prevents `https://app.com.evil.com/...`
4. **Path prefix match** — `https://app.com/auth/` doesn't allow `https://app.com/admin/...`
5. **Customer backend re-verifies** via `GET /v1/verifications/{id}` — doesn't rely solely on URL trust
6. **Token in URL is single-use** — attacker who intercepts a used URL gets 410

### 6.3 Logging

Every response includes:
- `account_id`, `api_key_id` (where applicable)
- `verification_id` (when known)
- `channel="magic_link"`
- `recipient_hash` (SHA-256 of email; never log plaintext email)
- `redirect_url_host` (host portion only; never log full URL with query)
- `request_id` (existing middleware)
- **Never log:** raw token plaintext, full redirect_url with query params

### 6.4 Prometheus metrics

Reuse + extend B1:

```
chorus_verifications_total{channel="magic_link", outcome="created|approved|expired|canceled|rate_limited|invalid_redirect"}
chorus_verifications_routing_total{chosen_channel="magic_link"}
chorus_verifications_cost_micro_total{channel="magic_link"}      # adds 100 per send/resend
chorus_verifications_create_duration_seconds (existing histogram)

# NEW
chorus_verifications_magic_link_callbacks_total{outcome="approved|expired_or_invalid|error"}
chorus_verifications_callback_duration_seconds (histogram)
```

No dedicated counter for "invalid_redirect" attempts — a WARN log line is enough; alert via Loki query if it spikes.

### 6.5 Sample email layout (reference)

```
From: noreply@chorus-mail.example.com
To: alice@app.com
Subject: Sign in to Acme

Click the link below to sign in to Acme:

https://api.chorus.example.com/v1/verifications/callback?token=k7P-y3FxLm…

This link expires in 1 hour and can only be used once.
If you didn't request this, ignore this email.
```

The "If you didn't request this" line is the anti-phishing safety net (industry standard since OAuth recommendations).

## 7. Testing Strategy

### 7.1 Unit tests (`verification.rs` extensions)

- `generate_magic_link_token_returns_43_char_base64url`
- `generate_magic_link_token_returns_32_byte_hash`
- `generate_magic_link_token_hash_matches_plaintext`
- `generate_magic_link_token_uses_full_entropy` (100 unique)
- `validate_redirect_url_accepts_exact_whitelist_match`
- `validate_redirect_url_accepts_path_prefix_match`
- `validate_redirect_url_rejects_different_host`
- `validate_redirect_url_rejects_host_suffix_attack`
- `validate_redirect_url_rejects_scheme_mismatch`
- `validate_redirect_url_rejects_port_mismatch`
- `validate_redirect_url_rejects_non_https_for_non_localhost`
- `validate_redirect_url_allows_http_localhost_for_dev`
- `validate_redirect_url_allows_http_127_0_0_1_for_dev`
- `validate_redirect_url_rejects_path_under_different_prefix`
- `validate_redirect_url_rejects_empty_whitelist`
- `validate_redirect_url_rejects_malformed_url`
- `build_magic_link_url_strips_trailing_slash_from_base`
- `append_verification_id_to_url_without_query`
- `append_verification_id_to_url_with_existing_query`

### 7.2 Repository tests (`sqlx::test`, ignored by default)

- `insert_magic_link_row_stores_token_hash`
- `find_by_id_returns_magic_link_with_redirect_url`
- `consume_magic_link_token_marks_approved_and_clears_hash`
- `consume_magic_link_token_returns_none_when_already_consumed`
- `consume_magic_link_token_returns_none_when_expired`
- `consume_magic_link_token_returns_none_when_canceled`
- `consume_magic_link_token_returns_none_for_unknown_hash`
- `consume_magic_link_token_is_atomic_under_concurrent_clicks` (5 spawned, exactly 1 succeeds)
- `magic_link_allowed_redirects_returns_array_from_accounts`
- `migration_011_relaxes_channel_check_constraint`
- `resend_replaces_token_hash_atomically`

### 7.3 Routing logic tests

`MockAccountRepo` extended; no DB / Valkey:

- `select_channel_returns_magic_link_when_only_email_and_magic_link_requested`
- `create_with_magic_link_requires_email_recipient`
- `magic_link_channel_validates_redirect_url_against_whitelist`
- `magic_link_channel_returns_invalid_redirect_when_not_https`
- `magic_link_channel_returns_redirect_not_whitelisted_when_no_match`
- `magic_link_channel_falls_back_to_account_default_when_no_request_url`

### 7.4 API integration tests (`tests/api_test.rs`)

Extend `MemVerificationRepo` + `MockAccountRepo` with `consume_magic_link_token` + `magic_link_allowed_redirects`:

- `create_magic_link_returns_201_with_pending_status_and_no_token_in_response`
- `create_magic_link_omits_token_hash_from_response_body`
- `create_magic_link_returns_invalid_redirect_when_url_not_https`
- `create_magic_link_returns_redirect_not_whitelisted_when_no_match`
- `create_magic_link_falls_back_to_account_default_redirect`
- `callback_with_valid_token_returns_302_to_redirect_url_with_verification_id_appended`
- `callback_appends_verification_id_to_url_with_existing_query`
- `callback_with_consumed_token_returns_410_html_landing`
- `callback_with_expired_token_returns_410_html_landing`
- `callback_with_canceled_token_returns_410_html_landing`
- `callback_with_garbled_token_returns_410_html_landing`
- `callback_with_missing_token_query_returns_400_html_landing`
- `callback_without_per_request_redirect_falls_back_to_account_default`
- `callback_with_no_account_default_returns_200_chorus_landing_page`
- `callback_landing_page_returns_text_html_content_type`
- `create_magic_link_idempotency_replay_returns_same_token`
- `resend_magic_link_generates_new_token_and_invalidates_old`
- `resend_does_not_extend_expires_at`
- `resend_max_3_returns_422_max_resends_reached`
- `rate_limit_per_recipient_blocks_6th_magic_link_create`
- `rate_limit_per_account_blocks_101st_call`
- `cancel_clears_magic_link_token_hash`
- `existing_sms_verification_works_after_migration_011`
- `existing_email_verification_works_after_migration_011`

### 7.5 Smoke test on podman

```bash
# 1. Boot stack with public_base_url
podman run -d --name chorus-ml-pg ... postgres:16-alpine
podman run -d --name chorus-ml-vk ... docker.io/valkey/valkey:8-alpine

KEY=$(head -c 32 /dev/urandom | base64)
DATABASE_URL=postgres://chorus:chorus@localhost:5433/chorus \
REDIS_URL=redis://127.0.0.1:6380 PORT=3001 HOST=127.0.0.1 \
CHORUS_ENCRYPTION_KEY="$KEY" \
CHORUS_PUBLIC_BASE_URL="http://127.0.0.1:3001" \
cargo run -p chorus-server &

# 2. Seed account WITH whitelist + api_key
podman exec -i chorus-ml-pg psql -U chorus -d chorus <<SQL
INSERT INTO accounts (id, name, owner_email, is_active, magic_link_allowed_redirects)
  VALUES ('00000000-...01', 'smoke', 's@x.com', true,
          ARRAY['http://localhost:8080/']);
INSERT INTO api_keys ...;
SQL

# 3. Create magic-link verification
RESP=$(curl -s -H "authorization: Bearer $SMOKE_KEY" -H "content-type: application/json" \
  -d '{"email":"alice@app.com","channels":["magic_link"],
       "redirect_url":"http://localhost:8080/welcome","app_name":"Smoke"}' \
  http://127.0.0.1:3001/v1/verifications)
ID=$(echo "$RESP" | python3 -c "import json,sys;print(json.load(sys.stdin)['id'])")

# 4. Read token from the queued message body (test-mode messages table)
BODY=$(podman exec -i chorus-ml-pg psql -U chorus -t -d chorus -c \
  "SELECT body FROM messages WHERE recipient='alice@app.com' ORDER BY created_at DESC LIMIT 1")
TOKEN=$(echo "$BODY" | grep -oE 'token=[A-Za-z0-9_-]+' | head -1 | cut -d= -f2)

# 5. Simulate click → 302
curl -s -i "http://127.0.0.1:3001/v1/verifications/callback?token=$TOKEN" | head -10
# expect: HTTP/1.1 302 Found; location: http://localhost:8080/welcome?verification_id=<uuid>

# 6. Server-side verify
curl -s -H "authorization: Bearer $SMOKE_KEY" \
  "http://127.0.0.1:3001/v1/verifications/$ID" | grep status
# expect: "status": "approved"

# 7. Replay click → 410
curl -s -i "http://127.0.0.1:3001/v1/verifications/callback?token=$TOKEN" | head -5
# expect: HTTP/1.1 410 Gone; Content-Type: text/html

# 8. Phishing attempt → 400
curl -s -H "authorization: Bearer $SMOKE_KEY" -H "content-type: application/json" \
  -d '{"email":"bob@app.com","channels":["magic_link"],
       "redirect_url":"https://evil.com/steal"}' \
  http://127.0.0.1:3001/v1/verifications
# expect: 400 {"error":{"code":"redirect_not_whitelisted",...}}

# 9. /metrics
curl -s http://127.0.0.1:3001/metrics | grep -E "magic_link"
```

## 8. Follow-up Criteria (Out of Scope — Trigger Conditions)

| Trigger | Follow-up |
|---|---|
| ≥3 customers request branded email | **B5.1** per-account email template (HTML + plaintext, minijinja) |
| Customer asks "notify my backend when user clicks" instead of polling | **B5.2** webhook `magic_link.approved` event |
| Customer requests `channels:["magic_link","sms"]` fallback | **B5.3** cross-channel waterfall extending B1 |
| Customer asks admin endpoint to manage `magic_link_allowed_redirects` | **B5.4** admin CRUD |
| Customer asks branded landing page | **B5.5** template upload + render |
| Need for open/click analytics | **B5.6** tracking pixel + click counters |
| `chorus_verifications_magic_link_callbacks_total{outcome="expired_or_invalid"}` > 30 % of `approved` rate | Increase TTL to 24 h, or surface "Resend link" hint in landing page |
| Customer reports "user gets verification_id but our backend always sees `pending`" | Race-condition investigation (browser fast-redirect + backend slow-poll) — consider returning a one-time confirmation token in the redirect instead of the verification_id |

## 9. Implementation Order

Each step = one commit (subagent-driven dev pattern):

1. Migration `011_add_magic_link.sql`.
2. `db::mod` types extension — `NewVerification` fields + `MagicLinkConsumeResult` + trait method signatures.
3. `PgVerificationRepository` impl — `consume_magic_link_token` (atomic UPDATE+RETURNING), extend `insert`/`record_resend` to handle new columns; extend sqlx tests (5 new cases).
4. `PgAccountRepository::magic_link_allowed_redirects` + sqlx test.
5. `verification.rs` constants + `generate_magic_link_token` + `build_magic_link_url` + `append_verification_id` + unit tests.
6. `verification.rs` `validate_redirect_url` + `RoutingError` variants + 14 unit tests.
7. `ChannelChoice::MagicLink` + `select_channel` extension + routing logic tests.
8. `Config::public_base_url` + env loading + AppState; README updates.
9. Routes: extend `create_verification_inner` for magic_link branch (validate + generate + insert + URL + email body).
10. Routes: NEW `callback_verification` handler + `chorus_landing_page` helper + wire route in `app.rs`.
11. Routes: extend `enqueue_verification_send` signature for optional magic_link_url; magic-link email subject + body templates.
12. API integration tests (1/2) — `MemVerificationRepo`+`MockAccountRepo` extensions; create/callback happy + validation (12).
13. API integration tests (2/2) — idempotency replay, resend invalidates old, cancel clears hash, rate-limit + B1 regression (15+).
14. Prometheus metrics — `chorus_verifications_magic_link_callbacks_total` + `chorus_verifications_callback_duration_seconds`.
15. CI sweep — fmt + clippy + test + deny; README adds `CHORUS_PUBLIC_BASE_URL` + `magic_link_allowed_redirects` notes.
16. Smoke test on podman (§7.5) + open PR.

A subsequent implementation plan via `superpowers:writing-plans` breaks each step into commit-sized tasks with exact code blocks and file paths.
