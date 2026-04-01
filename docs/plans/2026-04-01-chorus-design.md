# Chorus — Open-Source CPaaS Design Document

> **Date:** 2026-04-01
> **Status:** Approved
> **Author:** mrbt + Claude

## Overview

Chorus is an open-source Communications Platform as a Service (CPaaS) built in Rust. It provides SMS and Email delivery with smart routing, multi-provider failover, cost optimization, and SDKs for multiple languages.

**Primary consumers:** Nucleus (Rust auth platform) and Orbit (Java/Spring Boot finance API).

**Business model:** MIT-licensed self-hosted (free) + managed cloud service (subscription + usage-based pricing).

## Goals

1. Solve Nucleus/Orbit pain point: no SMS/Email delivery infrastructure
2. Smart cost optimization via waterfall channel selection (email-first, SMS-fallback)
3. Provider-agnostic: swap Telnyx/Twilio/Resend without code changes
4. Language-agnostic: Rust library (Phase 1) + REST API with SDKs (Phase 2)
5. Production-grade: async queue, retry, audit trail, billing

## Architecture

### Approach: Monolith + Library (A+C)

- **Phase 1:** Rust crate (`chorus-core` + `chorus-providers`) — Nucleus imports directly
- **Phase 2:** Axum REST API (`chorus-server`) — Orbit and other languages call via HTTP/SDKs

```
chorus/
├── crates/
│   ├── chorus-core/          # Traits, routing, types, errors
│   ├── chorus-providers/     # Telnyx, Twilio, Plivo, Resend, SES, SMTP, Mock
│   └── chorus-server/        # Axum REST API (Phase 2)
├── sdks/
│   ├── rust/                 # Native (uses chorus-core directly)
│   ├── typescript/
│   ├── go/
│   ├── java/
│   ├── python/
│   └── c/
├── dashboard/                # React + Vite (built into chorus-server)
├── Cargo.toml
├── Dockerfile
├── docker-compose.yml
└── LICENSE (MIT)
```

### System Architecture

```
SDKs (client-side)              Server (your infra)
┌──────────────┐              ┌──────────────────────┐
│ Rust SDK     │─────────────→│                      │
│ TS/JS SDK    │─────────────→│  chorus-server       │
│ Go SDK       │─────────────→│  (Axum REST API)     │
│ Java SDK     │─────────────→│                      │
│ Python SDK   │─────────────→│  ┌────────────────┐  │
│ C SDK        │─────────────→│  │ Async Queue    │  │
└──────┬───────┘              │  │ (Redis)        │  │
       │ audit trail          │  └───────┬────────┘  │
       └─────────────────────→│          │           │
                              │  ┌───────▼────────┐  │
                              │  │ Workers        │  │
                              │  │ Multi-provider │  │
                              │  │ fan-out        │  │
                              │  └───────┬────────┘  │
                              │          │           │
                              │  PostgreSQL + Redis  │
                              │  Prometheus          │
                              └──────────┬───────────┘
                                         │
                               ┌─────────▼──────────┐
                               │ Providers           │
                               │ Telnyx / Twilio     │
                               │ Resend / SES / SMTP │
                               └─────────────────────┘
```

## Tech Stack

| Layer | Tech | Reason |
|-------|------|--------|
| Language | Rust | Same as Nucleus, performance |
| Web framework | Axum | Same as Nucleus |
| Database | PostgreSQL | Message history, accounts, billing |
| Queue | Redis | Async job queue, caching |
| Metrics | `metrics` crate → Prometheus | Industry standard |
| Logs | `tracing` crate → stdout (→ Loki) | Rust standard |
| Dashboard | Strata (separate repo) | Full control, no Grafana |
| HTTP client | reqwest | Provider API calls |
| Container | Docker + Compose | Easy deploy |
| Payment | Stripe + Stripe Tax | Subscription billing + VAT |

## Built-in Providers

### SMS (Priority order)

| Priority | Provider | Reason |
|----------|----------|--------|
| 1 | Telnyx | Cheapest global, best DX |
| 2 | Twilio | Most popular, easy migration |
| 3 | Plivo | Good alternative |
| 4 | Mock/Log | Development mode |

### Email (Priority order)

| Priority | Provider | Reason |
|----------|----------|--------|
| 1 | Resend | Free 3,000/mo, modern API |
| 2 | AWS SES | Cheapest at scale |
| 3 | SMTP | Universal fallback |
| 4 | Mock/Log | Development mode |

## Core Traits

```rust
// chorus-core/src/sms.rs
#[async_trait]
pub trait SmsSender: Send + Sync {
    fn provider_name(&self) -> &str;
    async fn send(&self, msg: &SmsMessage) -> Result<SendResult, ChorusError>;
    async fn check_status(&self, id: &str) -> Result<DeliveryStatus, ChorusError>;
}

// chorus-core/src/email.rs
#[async_trait]
pub trait EmailSender: Send + Sync {
    fn provider_name(&self) -> &str;
    async fn send(&self, msg: &EmailMessage) -> Result<SendResult, ChorusError>;
}
```

## Waterfall Channel Selection

```
OTP Request
  → ① Email (free)         ← 60% resolve here
  → ② SMS provider #1      ← 25% resolve here
  → ③ SMS provider #2      ← fallback
  → ④ All failed → error
```

Reduces SMS cost by 60-80% for typical auth workloads.

## Data Model

### Core Tables

```sql
-- Accounts
accounts (id, name, owner_email, plan, is_active, created_at, updated_at)

-- API Keys (multiple per account, live/test environments)
api_keys (id, account_id, name, key_hash, key_prefix, environment, last_used_at,
          expires_at, is_revoked, created_at)
  -- Format: ch_live_a1b2c3d4 / ch_test_x9y8z7w6
  -- Test mode: log only, no real sending, free

-- Provider Configs (per account, ordered by priority)
provider_configs (id, account_id, channel, provider, credentials_enc, priority,
                  is_active, created_at)

-- Messages (every message sent)
messages (id, account_id, api_key_id, template_id, channel, provider, sender,
          recipient, subject, body, template_variables, status, provider_message_id,
          error_message, cost_microdollars, attempts, environment, created_at,
          delivered_at)

-- Delivery Events (timeline per message)
delivery_events (id, message_id, status, provider_data, created_at)

-- Audit Logs (SDK telemetry)
audit_logs (id, account_id, message_id, sdk_language, sdk_version, event,
            latency_ms, metadata, created_at)
```

### Template Tables

```sql
-- Built-in Template Collection (managed by Chorus)
template_collection (id, slug, name, category, description, thumbnail_url,
                     subject, html_body, text_body, variables, is_active, created_at)
  -- Categories: authentication, notification, billing
  -- Built-in: OTP Modern, OTP Minimal, Magic Link, Password Reset, Welcome, etc.

-- User Templates (fork from collection or create from scratch)
email_templates (id, account_id, source_template_id, slug, name, subject,
                 html_body, text_body, variables, version, is_active,
                 created_at, updated_at)
```

### Billing Tables

```sql
-- Plans
plans (id, slug, name, price_cents, billing_period, sms_quota, email_quota,
       sms_overage_microdollars, email_overage_microdollars, max_templates,
       max_providers, max_api_keys, max_webhooks, audit_retention_days,
       is_active, created_at)

-- Subscriptions
subscriptions (id, account_id, plan_id, status, current_period_start,
               current_period_end, cancel_at_period_end, payment_provider,
               payment_provider_sub_id, created_at, updated_at)

-- Usage (reset per billing period)
usage (id, account_id, period_start, period_end, sms_count, email_count,
       sms_overage_count, email_overage_count, total_cost_microdollars,
       created_at, updated_at)

-- Invoices
invoices (id, account_id, subscription_id, period_start, period_end,
          subtotal_cents, tax_cents, tax_rate_percent, tax_country, total_cents,
          sms_overage_cents, email_overage_cents, status, stripe_invoice_id,
          paid_at, created_at)

-- Webhook Endpoints
webhook_endpoints (id, account_id, url, secret_hash, events, is_active, created_at)
```

## REST API

### Sending
```
POST   /v1/sms/send              Send SMS
POST   /v1/email/send            Send email (raw HTML or template)
POST   /v1/otp/send              Waterfall: email → SMS (smart routing)
POST   /v1/otp/verify            Verify OTP code
```

### Messages
```
GET    /v1/messages               List messages (paginated)
GET    /v1/messages/{id}          Message detail + delivery timeline
```

### Templates
```
GET    /v1/templates/collection   Browse built-in templates
GET    /v1/templates              List account's templates
POST   /v1/templates              Create/fork template
GET    /v1/templates/{slug}       Get template
PUT    /v1/templates/{slug}       Update template
DELETE /v1/templates/{slug}       Delete template
POST   /v1/templates/{slug}/preview  Preview with variables
```

### Providers
```
GET    /v1/providers              List configured providers
POST   /v1/providers              Add provider
PUT    /v1/providers/{id}         Update provider config/priority
DELETE /v1/providers/{id}         Remove provider
GET    /v1/providers/health       Health check all providers
```

### API Keys
```
GET    /v1/keys                   List API keys
POST   /v1/keys                   Create new key
DELETE /v1/keys/{id}              Revoke key
```

### Webhooks
```
GET    /v1/webhooks               List webhook endpoints
POST   /v1/webhooks               Subscribe to events
PUT    /v1/webhooks/{id}          Update
DELETE /v1/webhooks/{id}          Remove
POST   /v1/webhooks/{id}/test     Send test event
```

### Billing
```
GET    /v1/billing/plan           Current plan + usage
GET    /v1/billing/plans          Available plans
POST   /v1/billing/subscribe      Subscribe/upgrade (→ Stripe Checkout)
POST   /v1/billing/cancel         Cancel at period end
GET    /v1/billing/usage          Current period usage
GET    /v1/billing/invoices       Invoice history
POST   /v1/billing/webhook/stripe Stripe webhook receiver
```

### Analytics (for Strata dashboard)
```
GET    /v1/analytics/summary      Sent/delivered/failed + cost
GET    /v1/analytics/providers    Per-provider stats
GET    /v1/analytics/timeseries   Volume over time
```

## SDK Interface

All SDKs share the same interface pattern:

```typescript
// TypeScript example
const chorus = new Chorus({ apiKey: 'ch_live_xxx' })

// SMS
await chorus.sms.send({ to: '+66812345678', body: 'Hello' })

// Email (template)
await chorus.email.send({
  to: 'user@test.com',
  template: 'orbit-otp',
  variables: { code: '123456' }
})

// OTP (smart routing)
const otp = await chorus.otp.send({ to: '+66812345678' })
const result = await chorus.otp.verify({ otpId: otp.otpId, code: '123456' })
```

SDKs automatically report audit trail (latency, retries, errors) back to Chorus server.

## Template System

Three ways to send email:
1. **User template:** `template: "my-otp"` — forked from collection, customized
2. **Built-in collection:** `template: "@otp_modern"` — use directly
3. **Raw HTML:** `html: "<h1>...</h1>"` — full developer control

Non-tech users can browse collection → customize in dashboard → save with their own name.

## Billing & Pricing

| | Free | Pro | Enterprise |
|--|------|-----|-----------|
| Price/month | $0 | $29 | Custom |
| SMS/month | 100 | 10,000 | Unlimited |
| Email/month | 1,000 | 50,000 | Unlimited |
| SMS overage | Blocked | $0.05/SMS | Negotiated |
| Email overage | Blocked | $0.001/email | Negotiated |
| Templates | 3 | Unlimited | Unlimited |
| Audit retention | 7 days | 90 days | 1 year |

VAT handled by Stripe Tax (Thailand 7%, auto-adjusts per country).

Self-hosted: free forever, no billing, user provides own provider keys.

## Scaling

```
1M messages → async queue + multi-provider fan-out

API (Axum):     100,000+ req/sec per instance
Queue (Redis):  Holds all jobs, persistent
Workers:        Fan-out to multiple providers
                4 workers × 100 msg/sec = 400 msg/sec
                1M messages ≈ 42 minutes

Horizontal:     Docker/K8s replicas
                40 workers = 4,000 msg/sec
                1M messages ≈ 4 minutes
```

## Monitoring

- **Data collection:** Prometheus (metrics) + Loki (logs) — self-hosted, free
- **Dashboard:** Strata (separate repo) — custom-built, full control
- **No Grafana** — replaced by Strata for full UI/UX control

## Deployment

```yaml
# docker-compose.yml
services:
  chorus:
    image: chorus:latest
    ports: ["3000:3000"]
  postgres:
    image: postgres:16
  redis:
    image: redis:7
  prometheus:
    image: prom/prometheus
```

Single Rust binary. Docker image ~20-50MB.

## Related Projects

- **Nucleus** (github.com/cntm-labs/nucleus) — Auth platform, primary Chorus consumer
- **Orbit** (github.com/cntm-labs/orbit-api) — Finance API, secondary Chorus consumer
- **Strata** (github.com/cntm-labs/strata) — Observability dashboard for Chorus (future)
