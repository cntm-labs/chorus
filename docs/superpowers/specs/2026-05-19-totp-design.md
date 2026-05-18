# TOTP / Authenticator App — Design

**Status:** Approved
**Date:** 2026-05-19
**Author:** chorus team
**Closes:** roadmap item B2 (differentiator tier #2 — "$0 TOTP verification")

---

## 1. Goal

Ship RFC 6238 TOTP enrollment + verification at `/v1/totp/*` — **$0 per verification** (vs Twilio Verify $0.013/check). Hero pitch of the B tier: "customers running 100k TOTP verifications/month save $1,300/month — no upper limit, no per-check cost."

Second deliverable of the **B (differentiator)** tier after B1 (Verification API + Waterfall). Together with B1, chorus now covers both message-delivered OTP (SMS/email) and authenticator-app OTP (TOTP) — full 2FA stack at fractions of Twilio Verify cost.

### Economics positioning

| Operation | chorus | Twilio Verify | Auth0 MFA | Saving |
|---|---|---|---|---|
| TOTP enroll | **$0** | $0.05 | $0.045 | 100 % |
| TOTP verify | **$0** | $0.013 | $0.012 | 100 % |
| Backup-code verify | **$0** | $0.013 | $0.012 | 100 % |

→ Opens the door for low-volume customers to migrate from Twilio Verify (no cost wall).

## 2. Scope

### In scope (B2)

- 5 primary endpoints under `/v1/totp/*`:
  - `POST /v1/totp/enroll` — generate secret + QR + backup codes; status `pending`
  - `POST /v1/totp/activate` — verify first code → status `active`
  - `POST /v1/totp/verify` — verify code post-activation (accepts TOTP or backup code)
  - `DELETE /v1/totp/{user_id}` — disenroll; clears secret
  - `GET /v1/totp/{user_id}` — status retrieval
- 1 secondary endpoint:
  - `POST /v1/totp/backup-codes/regenerate` — replace all backup codes
- 1 optional binary endpoint:
  - `GET /v1/totp/{user_id}/qr` — raw `image/png` for `<img src=...>` use cases
- End-user identity model: opaque `user_id` string (1–255 ASCII printable). First introduction of the concept to chorus.
- RFC 6238 defaults: SHA1 / 6 digits / 30 s period / ±1 step window.
- 2-step enrollment: `/enroll` → pending → `/activate` (first code) → active.
- 10 one-use backup recovery codes (10-char alphanumeric with hyphen). Hash-at-rest via SHA-256.
- AES-GCM-256 secret encryption at rest. Introduces `crypto.rs` module — first encrypted column in chorus.
- Single shared secret per user (multi-device = scan same QR on N devices).
- QR generation: `otpauth://` URI + 256×256 PNG data URI (`fast_qr` crate).
- Rate limiting (reuse C1/B1 Lua sliding-window):
  - per user: 5 verifies/min, 10 activates/min, 10 enrolls-or-regenerates/min
  - per account: 100/min (umbrella)
- Idempotency-Key on `/enroll` + `/backup-codes/regenerate` (reuse C1).
- Pricing: `cost_micro: 0` in every response (hero pitch).
- Prometheus metrics: enrollments, activations, verifies (by outcome and method), disenrollments, gauge for backup codes remaining, duration histograms.

### Out of scope (deferred to follow-up specs)

- **B2.1** Per-device factors (Twilio-style multi-factor — separate row per device)
- **B2.2** Encryption-key rotation (`chorus rotate-encryption-key` admin command + DB re-encrypt job)
- **B2.3** WebAuthn / Passkey (separate namespace `/v1/webauthn/*`)
- **B2.4** `provider_configs` encryption — separate `chore(security)` PR reusing the new `crypto.rs`
- **B2.5** SMS recovery fallback (alternative to backup codes)
- SDK helpers for autofill (iOS / Android) — SDK-side work, not chorus-server
- Optional pending-enrollment cleanup task (rarely needed; pending users are harmless)

### Non-goals

- Not an identity provider — chorus stores TOTP, not user attributes, sessions, or login flow.
- Not HOTP (HMAC-based, counter-driven) — TOTP only in MVP; HOTP rare in practice.
- No device-fingerprint enrollment — one secret per user (see §3 decision).
- No "force-unenroll-all" admin endpoint — direct SQL is acceptable for the rare ops case.

## 3. Schema

New migration `services/chorus-server/src/db/migrations/010_create_totp.sql`:

```sql
CREATE TABLE totp_users (
    account_id        UUID NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    user_id           TEXT NOT NULL CHECK (length(user_id) BETWEEN 1 AND 255),
    secret            BYTEA NOT NULL,                         -- AES-GCM blob: nonce(12) || ct || tag(16)
    status            TEXT  NOT NULL CHECK (status IN ('pending','active','disabled')),
    algorithm         TEXT  NOT NULL DEFAULT 'SHA1',
    digits            SMALLINT NOT NULL DEFAULT 6,
    period_secs       SMALLINT NOT NULL DEFAULT 30,
    issuer            TEXT,
    label             TEXT,
    last_verified_at  TIMESTAMPTZ,
    created_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    activated_at      TIMESTAMPTZ,
    updated_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (account_id, user_id)
);

CREATE INDEX totp_users_account_status_idx ON totp_users (account_id, status);

CREATE TABLE totp_backup_codes (
    id          BIGSERIAL PRIMARY KEY,
    account_id  UUID NOT NULL,
    user_id     TEXT NOT NULL,
    code_hash   BYTEA NOT NULL,
    used_at     TIMESTAMPTZ,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    FOREIGN KEY (account_id, user_id)
        REFERENCES totp_users (account_id, user_id)
        ON DELETE CASCADE
);

CREATE INDEX totp_backup_codes_user_idx ON totp_backup_codes (account_id, user_id, used_at);
CREATE UNIQUE INDEX totp_backup_codes_hash_uniq ON totp_backup_codes (account_id, user_id, code_hash);
```

### Design notes

| Field / index | Reason |
|---|---|
| PK `(account_id, user_id)` | Single row per user → matches the "single shared secret" decision. Cascade on accounts cleans up on tenant deletion. |
| `secret` BYTEA | AES-GCM blob layout `nonce(12) ‖ ciphertext ‖ tag(16)`. A 20-byte TOTP secret encrypts to ~48 bytes. |
| `status` 3-value enum | `pending` after `/enroll`, `active` after `/activate`, `disabled` after `/disenroll`. Disabled rows are kept for audit but their secret is overwritten with `\\x00` padding. |
| `algorithm` / `digits` / `period_secs` | Stored (not hardcoded) so B2.x can flip values per-user without migration; locked to RFC defaults in MVP. |
| `issuer` / `label` | Per-user QR customization, stored at enroll so `GET /qr` can re-generate identical QR later. |
| `last_verified_at` | Analytics surface + debug aid. |
| `activated_at` | Distinguishes "enrolled but never activated" (stale, ignorable) from "active". |
| `(account_id, status)` index | Fast "list active TOTPs per account" for admin views. |
| `totp_backup_codes` separate table | One-row-per-code → cheap `used_at` updates; cascade FK clears on disenroll. |
| `code_hash` BYTEA(32) | SHA-256 of plaintext. Bcrypt/argon2 give no benefit on a 50-bit-entropy input and add latency. SHA-256 is constant-time enough for this size. |
| `(account_id, user_id, code_hash)` UNIQUE | Cheap defense against vanishingly unlikely hash collisions. |
| `(account_id, user_id, used_at)` index | Fast "unused backup codes for this user" lookup → drives `low_backup_codes` warning. |
| No separate `nonce` column | Nonce embedded in the `secret` blob — chorus convention going forward. |
| No PII fields (`phone` / `email`) | TOTP doesn't need contact. PII stays in the customer's system. |

### Capacity

- `totp_users`: ~250 bytes/row → 1 M users ≈ 240 MB. Trivial.
- `totp_backup_codes`: ~900 bytes/user (10 rows) → 1 M users ≈ 860 MB. Manageable.

### Encryption summary (full impl in §4)

- Key source: env `CHORUS_ENCRYPTION_KEY` = base64(32 random bytes). **Required at startup; server panics if missing or malformed.** Dev key generation: `head -c 32 /dev/urandom | base64`.
- Cipher: `Aes256Gcm` from `aes-gcm = "0.10"` (already in workspace; unused until B2).
- Nonce: 12 random bytes per encrypt, prepended to ciphertext.
- AAD: none in MVP — single-key envelope encryption only. Add AAD only when B2.x integrity-binds to row identifiers.

### Valkey keys (rate-limit only)

```
totp:rl:verify:{account_id}:{user_id_hash}     ZSET, sliding 1 min, limit  5
totp:rl:activate:{account_id}:{user_id_hash}   ZSET, sliding 1 min, limit 10
totp:rl:enroll:{account_id}:{user_id_hash}     ZSET, sliding 1 min, limit 10
totp:rl:acct:{account_id}                      ZSET, sliding 1 min, limit 100
```

No code cache — TOTP codes are computed deterministically from `(secret, time)` every verify; no Valkey storage needed (unlike OTP which cached 5-minute codes).

## 4. Components

### 4.1 `crypto.rs` — encryption module (new, reusable)

```rust
//! AES-GCM-256 envelope encryption keyed from env `CHORUS_ENCRYPTION_KEY`.
//! Blob layout: nonce(12) || ciphertext || tag(16).

pub struct Encryptor { /* Aes256Gcm */ }

impl Encryptor {
    pub fn from_env() -> Result<Self, anyhow::Error>;       // fail-fast at startup
    pub fn encrypt(&self, plaintext: &[u8]) -> Result<Vec<u8>>;
    pub fn decrypt(&self, blob: &[u8])      -> Result<Vec<u8>>;
}
```

Lives in `services/chorus-server/src/crypto.rs`. May migrate to `chorus-core` in B2.4 when other features need it.

### 4.2 `db::totp` — repository layer

Types in `db/mod.rs`:

```rust
#[derive(Debug, Clone, sqlx::FromRow, Serialize, Deserialize)]
pub struct TotpUser {
    pub account_id: Uuid,
    pub user_id: String,
    #[serde(skip)] pub secret: Vec<u8>,        // never serialized
    pub status: String,
    pub algorithm: String,
    pub digits: i16,
    pub period_secs: i16,
    pub issuer: Option<String>,
    pub label: Option<String>,
    pub last_verified_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub activated_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
}

pub struct NewTotpUser {
    pub account_id: Uuid,
    pub user_id: String,
    pub encrypted_secret: Vec<u8>,
    pub issuer: Option<String>,
    pub label: Option<String>,
}

#[async_trait]
pub trait TotpRepository: Send + Sync {
    async fn enroll(&self, u: &NewTotpUser, backup_hashes: &[Vec<u8>]) -> Result<TotpUser, DbError>;
    async fn find(&self, account_id: Uuid, user_id: &str) -> Result<Option<TotpUser>, DbError>;
    async fn activate(&self, account_id: Uuid, user_id: &str) -> Result<(), DbError>;
    async fn touch_last_verified(&self, account_id: Uuid, user_id: &str) -> Result<(), DbError>;
    async fn disenroll(&self, account_id: Uuid, user_id: &str) -> Result<bool, DbError>;
    async fn consume_backup_code(&self, account_id: Uuid, user_id: &str, hash: &[u8]) -> Result<bool, DbError>;
    async fn unused_backup_codes_count(&self, account_id: Uuid, user_id: &str) -> Result<i64, DbError>;
    async fn replace_backup_codes(&self, account_id: Uuid, user_id: &str, hashes: &[Vec<u8>]) -> Result<(), DbError>;
}
```

`enroll` runs INSERT user + N backup-code inserts inside one transaction. `consume_backup_code` is a single atomic `UPDATE ... WHERE used_at IS NULL ... RETURNING id`. `disenroll` overwrites `secret` with `\\x00` padding and sets `status='disabled'`.

### 4.3 `totp.rs` — orchestration module

Constants + pure helpers + Valkey rate-limit wrapper:

```rust
pub const TOTP_DIGITS: u32         = 6;
pub const TOTP_PERIOD_SECS: u64    = 30;
pub const TOTP_ALGORITHM: &str     = "SHA1";
pub const TOTP_WINDOW: i64         = 1;
pub const SECRET_BYTES: usize      = 20;
pub const BACKUP_CODE_COUNT: usize = 10;
pub const LOW_BACKUP_THRESHOLD: i64 = 3;

pub const RATE_LIMIT_VERIFY_PER_USER_MIN:   u32 = 5;
pub const RATE_LIMIT_ACTIVATE_PER_USER_MIN: u32 = 10;
pub const RATE_LIMIT_ENROLL_PER_USER_MIN:   u32 = 10;
pub const RATE_LIMIT_PER_ACCT_MIN:          u32 = 100;

pub fn generate_secret() -> [u8; SECRET_BYTES];
pub fn compute_totp(secret: &[u8], time_seconds: u64) -> String;
pub fn verify_totp_with_window(secret: &[u8], now_seconds: u64, code: &str) -> bool;
pub fn build_otpauth_uri(issuer: &str, label: &str, secret_base32: &str) -> String;
pub fn base32_no_pad(bytes: &[u8]) -> String;
pub fn qr_png_data_uri(otpauth_uri: &str) -> anyhow::Result<String>;
pub fn generate_backup_codes() -> Vec<String>;
pub fn hash_backup_code(code: &str) -> Vec<u8>;
pub fn is_backup_code_format(s: &str) -> bool;
pub async fn check_rate_limits(redis: &redis::Client, account_id: Uuid, user_id_hash: &str, kind: RateLimitKind) -> Result<(), TotpError>;
```

Crates added:
- `totp-lite = "2"` — HMAC-SHA1 TOTP, ~60 LOC, no transitive deps (MIT).
- `fast_qr = "0.13"` — QR encoder + PNG renderer, BSD-3, ~12 KB compiled, no deps.
- `base32 = "0.5"` — RFC 4648 base32, MIT.

### 4.4 Routes — new file `routes/totp.rs`

6 + 1 handlers mirroring B1's route style (`Bytes` body + `idempotency::begin/finalize_and_respond` on idempotent paths):

```rust
pub async fn enroll_totp(...);              // POST /v1/totp/enroll — idempotent
pub async fn activate_totp(...);            // POST /v1/totp/activate
pub async fn verify_totp(...);              // POST /v1/totp/verify
pub async fn disenroll_totp(...);           // DELETE /v1/totp/{user_id}
pub async fn get_totp_status(...);          // GET /v1/totp/{user_id}
pub async fn regenerate_backup_codes(...);  // POST /v1/totp/backup-codes/regenerate — idempotent
pub async fn get_totp_qr(...);              // GET /v1/totp/{user_id}/qr → image/png raw
```

### 4.5 `AppState` wiring

Two new fields:

```rust
totp_repo: Arc<dyn TotpRepository>,
encryptor: Arc<Encryptor>,
```

With accessors, added to `with_repos` (11-arg signature), and constructed in `AppState::new` via `PgTotpRepository::new(db.clone())` and `Arc::new(Encryptor::from_env()?)`.

### 4.6 Background tasks

**None.** TOTP records are permanent until explicit `/disenroll`. Pending enrollments are harmless and rare; expiring them is a B2.x optional follow-up if metrics show drift.

## 5. Data Flow

### 5.1 Enroll (happy path)

```
POST /v1/totp/enroll
Headers: Idempotency-Key: req-abc
Body:    {"user_id":"alice@app.com","issuer":"Acme","label":"Acme:alice"}

→ idempotency::begin → Fresh
→ validate user_id (1-255 ASCII printable)
→ check_rate_limits(account, hash(user_id), Kind::Enroll)
→ existing = totp_repo.find
   → Some(active|pending) → 409 already_enrolled
   → Some(disabled) | None → proceed
→ secret    = totp::generate_secret()         // 20 bytes
→ encrypted = encryptor.encrypt(&secret)      // ~48 bytes
→ backup_plaintext = generate_backup_codes()  // 10 codes
→ backup_hashes    = backup_plaintext.map(hash_backup_code)
→ totp_repo.enroll(NewTotpUser, &backup_hashes)   // tx
→ otpauth = build_otpauth_uri(issuer, label, base32(secret))
→ qr_png  = qr_png_data_uri(otpauth)
→ Response 201:
   {
     "user_id": "alice@app.com",
     "status": "pending",
     "issuer": "Acme",
     "label":  "Acme:alice",
     "otpauth_uri": "otpauth://totp/Acme:alice?secret=...&issuer=Acme&algorithm=SHA1&digits=6&period=30",
     "qr_code_png": "data:image/png;base64,iVBORw0...",
     "backup_codes": ["a3f8-9d2cx", ...],   ⚠ plaintext, shown once
     "cost_micro": 0,
     "created_at": "..."
   }
```

### 5.2 Activate

```
POST /v1/totp/activate
Body: {"user_id":"alice@app.com","code":"483921"}

→ check_rate_limits(Kind::Activate)
→ user = totp_repo.find → guard status='pending'  (else 410 not_pending)
→ secret = encryptor.decrypt(user.secret)
→ verify_totp_with_window(&secret, now_unix(), "483921")
   → false → 422 incorrect_code
   → true  → totp_repo.activate → 200 updated user (status='active')
```

### 5.3 Verify (TOTP path)

```
POST /v1/totp/verify
Body: {"user_id":"alice@app.com","code":"731205"}

→ check_rate_limits(Kind::Verify)
→ user = totp_repo.find → guard status='active'  (else 410 not_active)
→ if is_backup_code_format(code) → jump to §5.4
→ secret = encryptor.decrypt(user.secret)
→ valid  = verify_totp_with_window(&secret, now_unix(), code)
   → false → 422 incorrect_code
   → true  → totp_repo.touch_last_verified
              → unused = totp_repo.unused_backup_codes_count
              → 200:
                {
                  "verified": true, "method": "totp",
                  "low_backup_codes": unused < 3,
                  "cost_micro": 0, ...
                }
```

### 5.4 Verify (backup-code path)

```
(reached when is_backup_code_format(code) is true)

→ hash = hash_backup_code(code)
→ consumed = totp_repo.consume_backup_code(account, user_id, &hash)
   (atomic UPDATE ... WHERE used_at IS NULL ... RETURNING id)
   → false → 422 incorrect_code   (don't disclose "code was used")
   → true  → totp_repo.touch_last_verified
              → 200:
                {
                  "verified": true, "method": "backup_code",
                  "low_backup_codes": <new unused> < 3, ...
                }
```

### 5.5 Disenroll

```
DELETE /v1/totp/{user_id}
→ disenrolled = totp_repo.disenroll
   (UPDATE totp_users SET status='disabled', secret=\\x00 WHERE …;
    FK cascade clears backup_codes)
→ false → 410 not_found_or_already_disabled
→ true  → 200 {"status":"disabled","disabled_at":"..."}
```

### 5.6 Status retrieval

```
GET /v1/totp/{user_id}
→ user = totp_repo.find
→ None  → 404 not_found
→ Some  → strip secret, return {user_id, status, issuer, label,
                                 last_verified_at, created_at, activated_at,
                                 unused_backup_codes_count}
```

### 5.7 Regenerate backup codes

```
POST /v1/totp/backup-codes/regenerate
Headers: Idempotency-Key: req-xyz
Body:    {"user_id":"alice@app.com"}

→ idempotency::begin → Fresh
→ check_rate_limits(Kind::Enroll)   // shared 10/min pool
→ user = totp_repo.find → guard status='active'
→ new_plaintext = generate_backup_codes()
→ new_hashes    = hashes
→ totp_repo.replace_backup_codes (tx: DELETE old + INSERT 10 new)
→ 200 with new plaintext backup_codes (shown once)
```

### 5.8 `GET /v1/totp/{user_id}/qr`

```
→ user = totp_repo.find → guard status in {pending, active}
→ secret  = encryptor.decrypt(user.secret)
→ otpauth = build_otpauth_uri(...)
→ png_bytes = render
→ 200 Content-Type: image/png, body = png_bytes raw
```

## 6. Error Handling + Metrics

### 6.1 HTTP error matrix

| Endpoint | Condition | HTTP | `error.code` |
|---|---|---|---|
| Enroll | missing / invalid user_id | 400 | `invalid_user_id` |
| Enroll | user already active or pending | 409 | `already_enrolled` |
| Enroll | rate limit (per-user 10/min) | 429 + `Retry-After` | `rate_limited` |
| Enroll | DB / encryption error | 500 | `internal` |
| Activate | unknown user | 404 | `not_found` |
| Activate | status != pending | 410 | `not_pending` |
| Activate | wrong code | 422 | `incorrect_code` |
| Activate | rate limit (10/min) | 429 + `Retry-After` | `rate_limited` |
| Verify | unknown user | 404 | `not_found` |
| Verify | status != active | 410 | `not_active` |
| Verify | wrong code (TOTP) | 422 | `incorrect_code` |
| Verify | wrong / already-used backup code | 422 | `incorrect_code` |
| Verify | rate limit (5/min) | 429 + `Retry-After` | `rate_limited` |
| Disenroll | user not found / already disabled | 410 | `not_found_or_already_disabled` |
| Status | user not found | 404 | `not_found` |
| Regenerate | status != active | 410 | `not_active` |
| Regenerate | rate limit | 429 + `Retry-After` | `rate_limited` |
| Any | per-account 100/min | 429 + `Retry-After` | `rate_limited` |
| Any | encryption key missing/wrong at runtime | 500 | `internal` (redacted log) |
| Idempotency | reused key, different body | 422 | `idempotency_key_reused` (from C1) |

### 6.2 Logging

- Every response includes `account_id`, `api_key_id`, `user_id_hash` (sha256), `request_id`.
- **Never log:** plaintext `secret`, plaintext backup codes, raw `code` parameter.
- Failed verify logs `code_length=6 method=totp` (or `method=backup`) — no actual code value.

### 6.3 Prometheus metrics

```
chorus_totp_enrollments_total{outcome="created|already_enrolled|rate_limited|error"}
chorus_totp_activations_total{outcome="activated|wrong_code|not_pending|rate_limited|error"}
chorus_totp_verifies_total{outcome="approved|wrong_code|rate_limited|error", method="totp|backup_code"}
chorus_totp_disenrollments_total{outcome="disabled|not_found"}
chorus_totp_backup_codes_remaining (gauge)
chorus_totp_enroll_duration_seconds  (histogram)
chorus_totp_verify_duration_seconds  (histogram)
```

No `cost_micro_total` metric since cost = 0 (already pitched in §1).

## 7. Testing Strategy

### 7.1 Unit tests — `crypto.rs`
- Round-trip empty / 1-byte / 32-byte / 20-byte (TOTP secret) plaintext.
- Decrypt rejects short blob, tampered ciphertext, wrong key.
- Encrypt produces unique nonces (100 distinct blobs for same plaintext).
- `from_env` rejects missing var, short key, malformed base64.

### 7.2 Unit tests — `totp.rs`
- `compute_totp` matches RFC 6238 §B test vectors (3 known outputs).
- `verify_totp_with_window` accepts current / −1 / +1 step; rejects ±2 steps and wrong codes.
- `generate_secret` is 20 bytes and unique across 100 calls.
- `base32_no_pad` matches RFC 4648 vectors.
- `build_otpauth_uri` matches format and escapes special chars.
- `generate_backup_codes` returns 10 unique codes in `[a-z0-9]{4}-[a-z0-9]{5}` format.
- `is_backup_code_format` true positives / true negatives.
- `hash_backup_code` deterministic and 32 bytes.
- `qr_png_data_uri` returns a `data:image/png;base64,` string whose decoded bytes are a valid PNG.

### 7.3 Repository tests (`sqlx::test`, ignored by default)
- `enroll_creates_pending_user_with_backup_codes`
- `enroll_errors_when_user_already_active`
- `enroll_reuses_slot_when_user_was_disabled`
- `find_returns_none_for_unknown_user`, `find_scopes_to_account`
- `activate_pending_to_active`, `activate_errors_when_not_pending`
- `touch_last_verified_updates_only_active`
- `disenroll_clears_secret_and_cascades_backup_codes`
- `consume_backup_code` marks-used / returns-false-when-already-used / returns-false-when-unknown
- `unused_backup_codes_count_excludes_used`
- `replace_backup_codes_atomically_swaps`
- `cascade_delete_on_account_removal_drops_user_and_codes`

### 7.4 API integration tests (`MockTotpRepo` + `MemIdempotencyRepo` pattern)
- Enroll: returns QR + backup codes; 409 when already enrolled; 400 on bad user_id; 429 on rate limit; idempotency replay byte-identical.
- Activate: correct → 200 active; wrong → 422; already active → 410; unknown → 404.
- Verify: TOTP success / TOTP wrong / backup-code success / backup-code reuse fails / pending user → 410 / `low_backup_codes: true` when under threshold.
- Disenroll: success path / second call → 410 / clears secret in DB.
- Status: returns metadata sans secret / unknown → 404.
- Regenerate: returns new codes, invalidates old, idempotency replay.
- Rate limits isolated per-user and per-account.

### 7.5 Encryption integration assertion
- After `enroll`, querying `totp_users.secret` directly shows an opaque blob (`octet_length ≈ 48`, not 20); decrypting with the configured key recovers the original secret.

### 7.6 Smoke test on podman

Adds `CHORUS_ENCRYPTION_KEY=$(head -c 32 /dev/urandom | base64)` to the B1 smoke recipe. Scenarios:

1. `enroll` → `status:"pending"`, plaintext `backup_codes` (10), `cost_micro:0`.
2. Compute current TOTP code from extracted secret (Python helper script).
3. `activate` with computed code → `status:"active"`.
4. `verify` with fresh TOTP code → `verified:true, method:"totp"`.
5. `verify` with one backup code → `verified:true, method:"backup_code"`.
6. Replay the same backup code → 422 `incorrect_code`.
7. Loop six wrong-code verifies → first five 422, sixth 429.
8. Postgres inspection: `octet_length(secret) ≈ 48`; backup-codes row count 10, used count 1.
9. `/metrics` exposes `chorus_totp_*` counters and histograms.

## 8. Follow-up Criteria (Out of Scope — Trigger Conditions)

| Trigger | Follow-up |
|---|---|
| Enterprise customer asks "let me name each authenticator app I've registered" | **B2.1** per-device factors (separate `totp_factors` table, `factor_id` first-class) |
| Encryption-key compromise or annual key rotation policy | **B2.2** `chorus rotate-encryption-key` admin command + DB re-encrypt migration |
| Customer requests passkey / WebAuthn 2FA | **B2.3** separate spec under `/v1/webauthn/*` |
| Self-hosted user requests fulfilment of CLAUDE.md "provider credentials encrypted at rest" promise | **B2.4** `chore(security)` PR — re-encrypt `provider_configs.credentials` reusing `crypto.rs` |
| Customer asks "let me send recovery codes by SMS instead" | **B2.5** SMS recovery fallback |
| `chorus_totp_verifies_total{outcome="rate_limited"}` > 1 % of traffic | Tune per-user rate-limit or expose as account-level config |
| `chorus_totp_backup_codes_remaining{user_id=...}` chronically near 0 across many users | Surface low-codes warning in account dashboard / proactive email |

## 9. Implementation Order

Each step = one commit (subagent-driven dev pattern, per B1):

1. Migration `010_create_totp.sql` (tables + indexes + FKs).
2. `crypto.rs` — `Encryptor` struct + `from_env` + `encrypt` / `decrypt` + 9 unit tests.
3. `totp.rs` (core) — consts, `generate_secret`, `compute_totp`, `verify_totp_with_window`, `base32_no_pad`, `build_otpauth_uri`, RFC 6238 test vectors.
4. `totp.rs` (backup codes + QR) — `generate_backup_codes`, `hash_backup_code`, `is_backup_code_format`, `qr_png_data_uri`.
5. `db::totp` — `TotpUser`, `NewTotpUser`, `TotpBackupCode` types + `TotpRepository` trait in `db/mod.rs`.
6. `PgTotpRepository` impl + 16 `sqlx::test` cases (ignored by default).
7. `AppState` wiring — `totp_repo` + `encryptor` fields, accessors, `with_repos` (11 args), `NullTotpRepo` for tests, thread through 3 existing call sites, load `CHORUS_ENCRYPTION_KEY` in `AppState::new`.
8. `totp.rs` rate-limit wrapper (`TotpError`, `RateLimitKind`, `check_rate_limits`) reusing the B1 Lua script.
9. Routes (1/2) — `routes/totp.rs` with `enroll`, `activate`, `verify`, `get_status`, `disenroll` + 5 route entries in `app.rs`.
10. Routes (2/2) — `regenerate_backup_codes`, `get_totp_qr` + 2 more route entries.
11. API integration tests (1/2) — `MockTotpRepo` + 12 tests: enroll/activate/disenroll/status/idempotency happy paths and validation errors.
12. API integration tests (2/2) — 12+ tests: verify (TOTP + backup), regenerate, rate limits, encryption-at-rest assertion.
13. Prometheus metrics per §6.3.
14. CI sweep — `cargo fmt`, `cargo clippy --workspace -- -D warnings`, `cargo test --workspace`, `cargo deny check`. Document sample dev `CHORUS_ENCRYPTION_KEY` in README.
15. Smoke test on podman (§7.6) + open PR.

A subsequent implementation plan via `superpowers:writing-plans` breaks each step into commit-sized tasks with exact code blocks and file paths.
