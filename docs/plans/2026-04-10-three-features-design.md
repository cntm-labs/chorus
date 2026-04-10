# Design: Mailgun Provider + Webhooks + Batch Send

**Date:** 2026-04-10
**Issues:** #11, #16, #17
**Delivery:** 3 separate PRs

---

## Feature 1: Mailgun Email Provider (#11)

### Struct

```rust
pub struct MailgunEmailSender {
    api_key: String,
    domain: String,
    from: String,
    http_client: reqwest::Client,
    base_url: String, // default: https://api.mailgun.net, EU: https://api.eu.mailgun.net
}
```

### Implementation

- **Auth:** Basic auth (`api` : `{api_key}`)
- **Content-Type:** `multipart/form-data`
- **Endpoint:** `POST /v3/{domain}/messages`
- **Response:** `{ "id": "<msg-id>", "message": "Queued..." }`
- **EU support:** configurable `base_url`

### Integration points

1. `chorus-providers/src/email/mod.rs` — re-export
2. `chorus-server/src/queue/router_builder.rs` — `("email", "mailgun")` match arm
3. `router_builder::build_router_from_env` — `MAILGUN_API_KEY`, `MAILGUN_DOMAIN`, `MAILGUN_FROM`

### Tests

- wiremock: success path, API error, network error
- Verify Basic auth header format
- Verify multipart form fields

---

## Feature 2: Webhook Callbacks (#16)

### Database

```sql
CREATE TABLE webhooks (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    account_id  UUID NOT NULL REFERENCES accounts(id),
    url         TEXT NOT NULL,
    secret      TEXT NOT NULL,
    events      TEXT[] NOT NULL,
    active      BOOLEAN NOT NULL DEFAULT true,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);
```

### API Endpoints

| Method | Path | Description |
|--------|------|-------------|
| POST   | `/v1/webhooks` | Register (url, events) → returns id + secret |
| GET    | `/v1/webhooks` | List webhooks for account |
| DELETE | `/v1/webhooks/{id}` | Remove webhook |

### Webhook Delivery

1. Worker completes message → query matching webhooks (account + event)
2. POST payload to URL with headers:
   - `X-Chorus-Signature`: HMAC-SHA256(secret, body)
   - `X-Chorus-Event`: event type
   - `X-Chorus-Timestamp`: unix timestamp
3. Retry: 3 attempts, exponential backoff via Redis delayed queue

### Payload

```json
{
  "event": "message.delivered",
  "message_id": "msg_xxx",
  "channel": "sms",
  "provider": "telnyx",
  "status": "delivered",
  "timestamp": "2026-04-10T12:00:00Z"
}
```

### Event Types

`message.queued`, `message.sent`, `message.delivered`, `message.failed`

---

## Feature 3: Batch Send (#17)

### API Endpoints

| Method | Path | Description |
|--------|------|-------------|
| POST   | `/v1/sms/send-batch` | Batch SMS (max 100 recipients) |
| POST   | `/v1/email/send-batch` | Batch email (max 100 recipients) |

### Request (SMS example)

```json
{
  "from": "+1234567890",
  "recipients": [
    { "to": "+1111111111", "body": "Hello A" },
    { "to": "+2222222222", "body": "Hello B" }
  ]
}
```

### Response (202 Accepted)

```json
{
  "messages": [
    { "message_id": "uuid-1", "to": "+1111111111", "status": "queued" },
    { "message_id": "uuid-2", "to": "+2222222222", "status": "queued" }
  ]
}
```

### Processing

- Fully async: each recipient → individual `NewMessage` + `SendJob`
- Redis pipeline `LPUSH` for bulk enqueue
- Max 100 recipients per batch (constant)
- Reuses existing message + job infrastructure entirely

---

## Decisions Log

| Decision | Choice | Reason |
|----------|--------|--------|
| Webhook auth | HMAC-SHA256 signature | Industry standard, replay protection |
| Batch processing | Fully async, individual jobs | Reuse existing infra, YAGNI batch tracking |
| Delivery order | #11 → #16 → #17 | Independence, increasing complexity |
| PR strategy | 3 separate PRs | Easier review, no blocking |
