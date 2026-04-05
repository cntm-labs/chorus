# chorus-server Phase 2b Design — Queue + Workers

**Goal:** Replace the placeholder worker with production-ready async queue processing that delivers messages via chorus-core providers.

## Decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| Provider config | Hybrid (env defaults + per-account DB override) | Flexibility without forcing DB setup for simple deploys |
| Worker concurrency | Configurable pool (N workers, each BRPOP) | Simple, no semaphore needed, Redis atomic pop prevents double-processing |
| Retry backoff | Delayed queue (Redis sorted set) | Doesn't block worker slots during backoff wait |
| Dead letter queue | DB status + Redis DLQ | Queryable via API + inspectable/replayable via Redis |

## Architecture

### Queue Keys

```
chorus:jobs            — main work queue (LPUSH/BRPOP)
chorus:delayed         — retry backoff sorted set (ZADD with timestamp score)
chorus:dead_letters    — failed jobs after max retries (LPUSH)
```

### Job Flow

```
API (202 Accepted) → LPUSH chorus:jobs
                          ↓
                     Worker BRPOP
                          ↓
                  Build WaterfallRouter
                  (per-account config or env defaults)
                          ↓
                  Send via chorus-core
                    ╱           ╲
                success        fail
                   ↓              ↓
              status=         attempts < 3?
              "delivered"      ╱        ╲
                            yes          no
                             ↓            ↓
                        ZADD delayed   LPUSH dead_letters
                        (score=now     status="failed"
                         +2^attempt)
```

### Delayed Queue Poller

Separate tokio task that runs every 1 second:
1. `ZRANGEBYSCORE chorus:delayed -inf {now}` — find due jobs
2. `ZREM` each job (atomic, prevents double-move)
3. `LPUSH chorus:jobs` — re-enqueue for processing

### Worker Pool

- `WORKER_CONCURRENCY` env var (default: 4)
- Each worker is a separate tokio task running BRPOP loop
- Redis BRPOP is atomic — no coordination needed between workers

## Provider Config

### Storage (Hybrid)

**Global defaults:** Environment variables (`TELNYX_API_KEY`, `TWILIO_ACCOUNT_SID`, etc.)

**Per-account override:** New `provider_configs` table:

```sql
CREATE TABLE provider_configs (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    account_id UUID NOT NULL REFERENCES accounts(id),
    channel TEXT NOT NULL CHECK (channel IN ('sms', 'email')),
    provider TEXT NOT NULL,
    priority INTEGER NOT NULL,
    credentials JSONB NOT NULL,
    is_active BOOLEAN NOT NULL DEFAULT true,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (account_id, channel, provider)
);
```

Credentials stored as plaintext JSONB in Phase 2b. AES-GCM encryption deferred to later phase.

### Router Resolution

1. Query `provider_configs` for account + channel (ordered by priority)
2. If found → build `WaterfallRouter` from per-account configs
3. If empty → build from env var globals
4. If test mode → always use `MockSmsSender` / `MockEmailSender`

## API Endpoints

```
GET    /v1/providers          — list provider configs for account
POST   /v1/providers          — add provider config
DELETE /v1/providers/{id}     — remove provider config
```

## Out of Scope

- Credential encryption (AES-GCM) — later phase
- Cost tracking (microdollars) — later phase
- Rate limiting — later phase
- DLQ replay endpoint — helper function only, no route
