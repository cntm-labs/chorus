# chorus-server Design — Phase 2a: Core Server

> **Date:** 2026-04-05
> **Status:** Approved
> **Depends on:** chorus-core, chorus-providers

## Goal

Build the Axum REST API server for Chorus — the HTTP interface that SDKs and external services call. Phase 2a covers: API key auth, sending endpoints, async queue, message history, OTP, and Docker Compose dev environment.

## Decisions

| Decision | Choice | Reason |
|----------|--------|--------|
| Database | sqlx (migrate to sentinel later) | Async native, lightweight, swap via repository trait |
| Queue | redis-rs + custom job queue | Simple send/retry logic, no framework overhead |
| OTP storage | Redis with TTL | Auto-expire, no cleanup needed |
| Dev environment | Docker Compose | Easy onboarding for contributors |

## Architecture

```
chorus-server/src/
├── main.rs              # Axum app bootstrap
├��─ config.rs            # Env-based config (DATABASE_URL, REDIS_URL, etc.)
├── app.rs               # Router setup, shared AppState
├── auth/
│   ��── api_key.rs       # Middleware: extract & validate ch_live_/ch_test_ keys
├── routes/
│   ├── sms.rs           # POST /v1/sms/send
│   ├── email.rs         # POST /v1/email/send
│   ├��─ otp.rs           # POST /v1/otp/send, POST /v1/otp/verify
│   ├── messages.rs      # GET /v1/messages, GET /v1/messages/{id}
│   ��── keys.rs          # CRUD /v1/keys
│   └── health.rs        # GET /health
├── db/
│   ├─�� mod.rs           # Repository traits (swappable for sentinel)
│   ├── postgres.rs      # sqlx implementations
│   └── migrations/      # SQL migration files
├── queue/
���   ├── mod.rs           # Job trait
│   ├��─ redis.rs         # Redis-backed queue
│   └── worker.rs        # Worker loop: dequeue → send via chorus-core
└── otp/
    └── mod.rs           # Generate, store (Redis TTL), verify
```

## Database Layer — Repository Trait

Designed for future sqlx → sentinel migration:

```rust
#[async_trait]
pub trait MessageRepository: Send + Sync {
    async fn insert(&self, msg: &NewMessage) -> Result<Message, DbError>;
    async fn find_by_id(&self, id: Uuid) -> Result<Option<Message>, DbError>;
    async fn list_by_account(&self, account_id: Uuid, pagination: &Pagination) -> Result<Vec<Message>, DbError>;
}

#[async_trait]
pub trait AccountRepository: Send + Sync {
    async fn find_by_api_key_hash(&self, hash: &str) -> Result<Option<Account>, DbError>;
}
```

Business logic depends on traits, not sqlx directly.

## API Key Auth

- Format: `ch_live_<32hex>` (production) / `ch_test_<32hex>` (test mode)
- Stored as SHA-256 hash in `api_keys` table
- Axum middleware extracts `Authorization: Bearer <key>`, looks up hash
- Injects `AccountContext { account_id, environment, key_id }` into request extensions
- Test mode (`ch_test_`): logs message, stores in DB with `environment=test`, never calls providers

## Async Queue Flow

```
Client → POST /v1/sms/send
  → Auth middleware (validate API key)
  → Enqueue job to Redis (message_id, payload)
  → Insert message to DB (status: queued)
  → Return 202 { message_id, status: "queued" }

Worker loop:
  → Dequeue from Redis
  → Load provider config for account
  → Send via chorus-core (WaterfallRouter)
  → Update message status in DB
  → Insert delivery_event
  → On failure: retry up to 3 times with backoff
```

## OTP Flow

```
POST /v1/otp/send { to: "+66812345678" }
  → Generate 6-digit code
  → Store in Redis: otp:{hash(to)} → { code, attempts: 0 } TTL 5min
  → Send via waterfall (email if @, SMS if phone)
  → Return 200 { otp_id }

POST /v1/otp/verify { otp_id, code: "123456" }
  → Lookup Redis key
  → If expired → 410 Gone
  → If wrong code → increment attempts, max 3 → 429
  → If correct → delete key → 200 { verified: true }
```

## Database Tables (Phase 2a)

```sql
CREATE TABLE accounts (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name TEXT NOT NULL,
    owner_email TEXT NOT NULL UNIQUE,
    is_active BOOLEAN NOT NULL DEFAULT true,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE api_keys (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    account_id UUID NOT NULL REFERENCES accounts(id),
    name TEXT NOT NULL,
    key_hash TEXT NOT NULL UNIQUE,
    key_prefix TEXT NOT NULL,
    environment TEXT NOT NULL CHECK (environment IN ('live', 'test')),
    last_used_at TIMESTAMPTZ,
    expires_at TIMESTAMPTZ,
    is_revoked BOOLEAN NOT NULL DEFAULT false,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE messages (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    account_id UUID NOT NULL REFERENCES accounts(id),
    api_key_id UUID NOT NULL REFERENCES api_keys(id),
    channel TEXT NOT NULL CHECK (channel IN ('sms', 'email')),
    provider TEXT,
    sender TEXT,
    recipient TEXT NOT NULL,
    subject TEXT,
    body TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'queued',
    provider_message_id TEXT,
    error_message TEXT,
    cost_microdollars BIGINT NOT NULL DEFAULT 0,
    attempts INTEGER NOT NULL DEFAULT 0,
    environment TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    delivered_at TIMESTAMPTZ
);

CREATE TABLE delivery_events (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    message_id UUID NOT NULL REFERENCES messages(id),
    status TEXT NOT NULL,
    provider_data JSONB,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
```

## REST API Endpoints (Phase 2a)

| Method | Path | Description | Auth |
|--------|------|-------------|------|
| POST | /v1/sms/send | Send SMS | API key |
| POST | /v1/email/send | Send email | API key |
| POST | /v1/otp/send | Send OTP (waterfall) | API key |
| POST | /v1/otp/verify | Verify OTP code | API key |
| GET | /v1/messages | List messages (paginated) | API key |
| GET | /v1/messages/{id} | Message detail + events | API key |
| GET | /v1/keys | List API keys | API key |
| POST | /v1/keys | Create new key | API key |
| DELETE | /v1/keys/{id} | Revoke key | API key |
| GET | /health | Health check | None |

## Docker Compose

```yaml
services:
  chorus:
    build: .
    ports: ["3000:3000"]
    environment:
      DATABASE_URL: postgres://chorus:chorus@postgres:5432/chorus
      REDIS_URL: redis://redis:6379
    depends_on:
      postgres: { condition: service_healthy }
      redis: { condition: service_healthy }
  postgres:
    image: postgres:16
    environment:
      POSTGRES_USER: chorus
      POSTGRES_PASSWORD: chorus
      POSTGRES_DB: chorus
    ports: ["5432:5432"]
    healthcheck:
      test: pg_isready -U chorus
      interval: 5s
      timeout: 5s
      retries: 5
  redis:
    image: redis:7
    ports: ["6379:6379"]
    healthcheck:
      test: redis-cli ping
      interval: 5s
      timeout: 5s
      retries: 5
```

## Dependencies (new for chorus-server)

```toml
axum = "0.8"
tower-http = { version = "0.6", features = ["cors", "trace"] }
sqlx = { version = "0.8", features = ["runtime-tokio", "tls-native-tls", "postgres", "uuid", "chrono", "json"] }
redis = { version = "0.27", features = ["tokio-comp"] }
sha2 = "0.10"
dotenvy = "0.15"
```

## Future Phases

- **Phase 2b:** Templates CRUD, Providers management, provider_configs table
- **Phase 2c:** Billing (Stripe), plans, subscriptions, usage tracking, invoices
- **Phase 2d:** Webhooks, analytics endpoints
- **Phase 2e:** Migrate sqlx → sentinel-driver
