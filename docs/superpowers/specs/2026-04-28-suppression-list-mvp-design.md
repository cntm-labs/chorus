# Suppression List MVP — Design

**Status:** Approved
**Date:** 2026-04-28
**Author:** chorus team
**Closes:** TODO at `services/chorus-server/src/routes/internal.rs:49-51`

---

## 1. Goal

Prevent Chorus from sending messages to recipients who have hard-bounced (via chorus-mail) or have been manually blocked by the account owner. Reject suppressed sends at the API entry point with HTTP 422, so no message is persisted and no billing is incurred.

This is the first deliverable in the **production-safety** tier (see follow-up specs for idempotency, external-provider bounce webhooks, and SMS STOP handling).

## 2. Scope

### In scope
- Per-account, per-channel suppression list keyed on `(account_id, channel, recipient)`.
- Two suppression sources for MVP:
  - **chorus-mail bounces** — extends the existing `/internal/bounces` handler, which currently logs and discards.
  - **Manual API** — customer-facing CRUD on `/v1/suppressions`.
- Hot-path lookup at every send route (`messages`, `sms`, `email`, `otp`, `batch`).

### Out of scope (deferred to follow-up specs)
- External provider bounce webhooks (SES, Resend, Mailgun).
- SMS STOP keyword detection (Twilio/Telnyx inbound webhook).
- Complaint / Feedback Loop (FBL) ingestion.
- `List-Unsubscribe` header support.
- TTL / auto-expiry of suppression entries.
- Admin-specific endpoints (admin can use raw SQL or hit customer endpoints with admin auth in a follow-up).
- Redis cache layer.
- Bulk import endpoint.

## 3. Schema

New migration `services/chorus-server/src/db/migrations/007_create_suppressions.sql`:

```sql
CREATE TABLE suppressions (
    account_id  UUID NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    channel     TEXT NOT NULL CHECK (channel IN ('sms', 'email')),
    recipient   TEXT NOT NULL,
    reason      TEXT NOT NULL CHECK (reason IN ('hard_bounce', 'manual')),
    source      TEXT NOT NULL CHECK (source IN ('chorus-mail', 'api')),
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (account_id, channel, recipient)
);
```

Notes:
- The composite primary key serves as the lookup index — no additional indexes for MVP.
- `recipient` is stored in normalized form (see §4).
- Reason and source are constrained CHECK enums; both are extensible in follow-up migrations.
- `ON DELETE CASCADE` on `account_id` so deleting an account cleans up its suppressions.

## 4. Recipient Normalization

A shared helper in `services/chorus-server/src/suppression.rs`:

```rust
pub fn normalize(channel: &str, recipient: &str) -> Result<String, NormalizeError> {
    match channel {
        "email" => Ok(recipient.trim().to_lowercase()),
        "sms"   => {
            // E.164: leading '+', country code 1-9, total 8-15 digits.
            let r = recipient.trim();
            let re = regex::Regex::new(r"^\+[1-9]\d{1,14}$").unwrap();
            if re.is_match(r) { Ok(r.to_string()) } else { Err(NormalizeError::InvalidE164) }
        }
        _ => Err(NormalizeError::UnknownChannel),
    }
}
```

- Email: lowercase + trim (no provider-specific magic like Gmail dot-stripping).
- SMS: validate E.164, store as-is.
- All paths (bounce ingest + manual API + hot-path lookup) call `normalize()` to guarantee a single canonical form.

## 5. Components

### 5.1 `db::suppression`

New file `services/chorus-server/src/db/suppression.rs` defining:

```rust
#[derive(Debug, Clone)]
pub struct Suppression {
    pub account_id: Uuid,
    pub channel: String,
    pub recipient: String,
    pub reason: String,
    pub source: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct SuppressionEntry {
    pub account_id: Uuid,
    pub channel: String,
    pub recipient: String,   // already normalized by caller
    pub reason: String,
    pub source: String,
}

#[async_trait]
pub trait SuppressionRepository: Send + Sync {
    async fn is_suppressed(
        &self,
        account_id: Uuid,
        channel: &str,
        recipient: &str,
    ) -> Result<Option<String>, DbError>;   // returns reason if suppressed

    async fn add(&self, entry: SuppressionEntry) -> Result<(), DbError>;  // ON CONFLICT DO NOTHING

    async fn remove(
        &self,
        account_id: Uuid,
        channel: &str,
        recipient: &str,
    ) -> Result<bool, DbError>;             // true if a row was deleted

    async fn list(
        &self,
        account_id: Uuid,
        channel: Option<&str>,
        pagination: &Pagination,
    ) -> Result<Vec<Suppression>, DbError>;
}
```

Postgres implementation in same file.

### 5.2 `suppression.rs` (hot-path helper)

New file `services/chorus-server/src/suppression.rs`:

```rust
pub enum SuppressionRejection {
    Suppressed { reason: String },
    InvalidRecipient,
    Db(DbError),
}

pub async fn check_suppression(
    state: &AppState,
    account_id: Uuid,
    channel: &str,
    recipient: &str,
) -> Result<(), SuppressionRejection> {
    let normalized = normalize(channel, recipient)
        .map_err(|_| SuppressionRejection::InvalidRecipient)?;

    match state.suppression_repo().is_suppressed(account_id, channel, &normalized).await {
        Ok(Some(reason)) => Err(SuppressionRejection::Suppressed { reason }),
        Ok(None) => Ok(()),
        Err(e) => Err(SuppressionRejection::Db(e)),
    }
}
```

Maps to HTTP responses:
- `Suppressed { reason }` → `422 { error: { code: "recipient_suppressed", reason } }`
- `InvalidRecipient` → `400 { error: { code: "invalid_recipient" } }`
- `Db(_)` → `500`

### 5.3 `/v1/suppressions` (customer API)

New file `services/chorus-server/src/routes/suppressions.rs`. All endpoints under existing `AccountContext` (api_key auth).

| Method | Path | Body / Query | Response |
|---|---|---|---|
| GET    | `/v1/suppressions` | `?channel=&limit=20&offset=0` | `{ data: [Suppression], limit, offset }` |
| POST   | `/v1/suppressions` | `{channel, recipient}` | `201` (or `200` if existed) — `reason` forced to `"manual"`, `source` forced to `"api"` |
| DELETE | `/v1/suppressions/{channel}/{recipient}` | — | `204` if removed, `404` if absent |

Validation:
- POST: `normalize()` first; bad E.164 → 400.
- POST: rejects client-supplied `reason` other than `"manual"` (force-override server-side rather than fail).
- GET: pagination follows existing `messages` route conventions (default 20, max 100).

### 5.4 `/internal/bounces` (extended)

Modify `services/chorus-server/src/routes/internal.rs::handle_bounce`:

```text
1. Validate shared secret (existing).
2. Strip <> from message_id (defensive — shell already does this).
3. SELECT id, account_id, channel, recipient FROM messages WHERE provider_message_id = $1
4. Not found → log warning + return 200 (postfix expects success, retries are wasteful here).
5. Found → normalize the looked-up recipient via suppression::normalize (canonical form),
   then in a single Postgres transaction:
   a. INSERT INTO suppressions (account_id, channel, recipient=normalized,
      reason='hard_bounce', source='chorus-mail') ON CONFLICT DO NOTHING
   b. UPDATE messages SET status='bounced' WHERE id = $1 AND status != 'bounced'
   c. INSERT INTO delivery_events (message_id, status, provider_data)
      VALUES ($1, 'bounced', jsonb_build_object('reason', $2, 'source', 'chorus-mail'))
6. Return 200.
```

Note: the suppression's `recipient` is normalized from `messages.recipient` (the value Chorus
originally accepted), not from `BounceNotification.recipient` (which postfix may rewrite via
aliasing). This keeps the suppression key consistent with future hot-path lookups.

The bounce-handler shell (`chorus-mail/scripts/bounce-handler.sh`) is unchanged — it already `exit 0`s on curl failure, so postfix is insulated.

### 5.5 Hot-path integration

Add `check_suppression(...)` call at the start of:
- `routes/messages.rs::create_message`
- `routes/sms.rs::send_sms`
- `routes/email.rs::send_email`
- `routes/otp.rs::send_otp`
- `routes/batch.rs::create_batch`

Single-recipient routes: rejection → return 422 immediately.

Batch route: filter per entry. Response shape changes from a single 202 to **207 Multi-Status** when at least one entry is suppressed:
```json
{
    "accepted": [{ "index": 0, "message_id": "uuid" }, { "index": 2, "message_id": "uuid" }],
    "suppressed": [{ "index": 1, "recipient": "x@y.com", "reason": "hard_bounce" }]
}
```
If all entries are accepted → existing 202 behavior is preserved.
If all entries are suppressed → 207 with empty `accepted`.

## 6. Data Flow

### 6.1 Send path
```
client → POST /v1/email
       → AccountContext extractor (auth)
       → check_suppression(account, channel="email", recipient)
            ├── normalized + lookup hits suppressions row → 422 (no message row, no billing)
            └── no row → existing send logic (queue → worker → provider)
```

### 6.2 Bounce path
```
recipient MTA bounces → postfix bounce daemon
                     → bounce-handler.sh (extracts Message-ID, diagnostic-code)
                     → POST /internal/bounces (with shared-secret header)
                     → handle_bounce
                          ├── secret invalid → 401
                          ├── message lookup fails → 200 + warn log
                          └── tx { suppress + mark bounced + delivery_event } → 200
```

### 6.3 Manual API path
```
customer → POST /v1/suppressions (api_key auth)
        → normalize → insert ON CONFLICT DO NOTHING
        → 201 (or 200 idempotent)

customer → DELETE /v1/suppressions/email/alice@example.com
        → normalize → delete returning rowcount
        → 204 if deleted, 404 otherwise
```

## 7. Error Handling

| Endpoint | Condition | Response |
|---|---|---|
| Any send route | recipient suppressed | `422` `{error: {code:"recipient_suppressed", reason:"hard_bounce"|"manual"}}` |
| POST `/v1/suppressions` | invalid format (E.164 fail / unknown channel) | `400` `{error: {code:"invalid_recipient"}}` |
| POST `/v1/suppressions` | already exists | `200` (idempotent), body returns existing entry |
| DELETE `/v1/suppressions/...` | row absent | `404` |
| `/internal/bounces` | unknown `provider_message_id` | `200` + structured log warning |
| `/internal/bounces` | DB tx error | `500` (postfix shell already swallows) |
| Any | DB connection failure | `500` |

All structured logs include `account_id`, `channel`, `recipient`, `request_id` (from PR #43 middleware) for Loki correlation.

## 8. Testing Strategy

### 8.1 Unit tests
- `suppression::normalize`:
  - Email: `"  Alice@Example.COM  "` → `"alice@example.com"`
  - SMS valid: `"+66812345678"` → unchanged
  - SMS invalid: `"0812345678"`, `"+0..."`, `"+abc"` → error

### 8.2 Repository tests (`sqlx::test` fixtures)
- `add` then `is_suppressed` returns `Some(reason)`
- `add` twice → second is no-op (ON CONFLICT)
- `remove` returns `true` once, then `false`
- `list` with channel filter
- `list` pagination

### 8.3 API integration tests (`tests/api_test.rs`)
- POST `/v1/messages` with suppressed recipient → 422 with correct error body
- POST `/v1/messages` with unsuppressed recipient → 202 (regression check)
- POST `/v1/suppressions` then POST `/v1/messages` → 422
- DELETE `/v1/suppressions/...` then POST `/v1/messages` → 202
- POST `/v1/suppressions` twice → second is idempotent (200, no duplicate row)
- POST `/v1/suppressions` with `"+0..."` → 400
- POST `/v1/suppressions` with `reason="hard_bounce"` from customer → server forces `"manual"`
- GET `/v1/suppressions?channel=email` filters correctly
- Batch with mixed suppressed/accepted recipients → 207 with partition

### 8.4 Bounce flow integration test
- Insert a message with `provider_message_id="<test123@chorus-mail>"`
- POST `/internal/bounces` with `message_id="test123@chorus-mail"` and valid secret
  - assert: suppression row exists for the message's `account_id` + `channel` + `recipient`
  - assert: message's `status` is `"bounced"`
  - assert: a `delivery_events` row exists with `status="bounced"` and `provider_data` includes the reason
- POST `/internal/bounces` with unknown `message_id` → 200, no suppression row written

## 9. Implementation Order

1. Migration `007_create_suppressions.sql`.
2. `db::suppression` repo trait + Postgres impl + tests.
3. `suppression.rs` (`normalize`, `check_suppression`, error types).
4. `/v1/suppressions` routes + tests.
5. Hot-path integration in `messages`, `sms`, `email`, `otp`.
6. Batch route partition response.
7. Extend `/internal/bounces` handler + tx + tests.
8. Final CI sweep (`cargo test --workspace`, clippy, fmt, deny).

A subsequent implementation plan (via `superpowers:writing-plans`) will break each of these into commit-sized tasks.
