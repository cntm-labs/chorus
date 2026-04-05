# Phase 3: Providers, Templates & chorus-mail — Design Document

> **Scope:** Resolves GitHub issues #6, #7, #8, #9, #10, #11 in a single unified plan.

## Overview

Phase 3 replaces stub implementations with production-ready features across three sub-phases:
- **3a:** Real SMS delivery status checking via provider APIs
- **3b:** Pluggable template engine (minijinja) + 5 built-in auth templates
- **3c:** Self-hosted email infrastructure (chorus-mail) + integration guide

## Decisions Log

| Topic | Decision | Rationale |
|-------|----------|-----------|
| SMS check_status | Full API implementation (Twilio, Telnyx, Plivo) | Delivery tracking + retry logic depend on accurate status |
| SendGrid (#6) | Close — use existing SmtpEmailSender | `smtp.sendgrid.net:587` works with current SMTP provider |
| Mailgun (#11) | Close — use existing SmtpEmailSender | `smtp.mailgun.org:587` works with current SMTP provider |
| Template engine | minijinja as default backend | Lightweight, Jinja2 syntax, zero-config, backward compatible with `{{var}}` |
| Pre-built templates | 5 minimal plain HTML templates | Easy to re-brand, low maintenance |
| Self-hosted email | chorus-mail: Postfix + OpenDKIM Docker image | Zero cost, full control, fits self-hosted philosophy |
| Self-hosted SMS | Not feasible — keep Telnyx/Twilio/Plivo | SMS requires telecom carrier (carrier-gated, not open protocol) |
| Bounce handling | Postfix pipe → HTTP webhook to chorus-server | Real-time bounce detection |
| Webhook auth | Shared secret via `X-Chorus-Secret` header | Defense in depth for internal endpoints |
| DNS tooling | CLI setup script + health check endpoint | Initial setup + ongoing verification |
| Integration guide | `docs/guides/auth-service-integration.md` | Separate from README, room for future guides |

---

## Phase 3a: SMS check_status() Real Implementation

### Problem

All SMS providers return hardcoded `DeliveryStatus::Sent` without calling provider APIs (#8).

### Solution

Implement real HTTP GET calls to each provider's status API.

### Files Modified

- `crates/chorus-providers/src/sms/telnyx.rs`
- `crates/chorus-providers/src/sms/twilio.rs`
- `crates/chorus-providers/src/sms/plivo.rs`

### Status API Endpoints

| Provider | Endpoint | Auth | Status Field |
|----------|----------|------|-------------|
| Twilio | `GET /2010-04-01/Accounts/{sid}/Messages/{id}.json` | Basic (sid:token) | `.status` |
| Telnyx | `GET /v2/messages/{id}` | Bearer token | `.data.to[0].status` |
| Plivo | `GET /v1/Account/{id}/Message/{uuid}/` | Basic (id:token) | `.message_state` |

### Status Mapping

```
Twilio:  queued/accepted/sending → Sent,  sent/delivered → Delivered,  failed/undelivered → Failed
Telnyx:  queued/sending          → Sent,  sent/delivered → Delivered,  sending_failed     → Failed
Plivo:   queued/sent             → Sent,  delivered      → Delivered,  failed/rejected    → Failed

Unknown status strings → Sent (safe default)
```

### Response Structs

Each provider gets a dedicated deserialization struct:
- `TwilioStatusResponse { status: String }`
- `TelnyxStatusResponse { data: { to: [{ status: String }] } }`
- `PlivoStatusResponse { message_state: String }`

### Estimated LOC: ~120-150

---

## Phase 3b: Template Engine + Built-in Templates

### Problem

Template engine only supports `{{variable}}` replacement (#10). No pre-built templates (#7).

### Solution

Replace internal render engine with minijinja. Ship 5 built-in auth templates.

### Template Engine

**Dependency:** `minijinja` crate added to `chorus-core/Cargo.toml`.

**Change:** Internal `replace_vars()` in `template.rs` delegates to minijinja. Public API (`Template::render()`) unchanged — backward compatible.

**New capabilities (zero extra code):**
- `{% if var %}...{% else %}...{% endif %}`
- `{% for item in list %}...{% endfor %}`
- `{{ var | upper }}`, `{{ var | default("N/A") }}` — filters

**Backward compatibility:** `{{ variable }}` syntax identical in Jinja2.

**Error handling:** Invalid template syntax → `ChorusError::Template` (instead of silently leaving placeholder).

### Built-in Templates

**New directory:** `crates/chorus-core/src/templates/`

| Slug | Variables | Subject |
|------|-----------|---------|
| `otp` | `code`, `app_name`, `expiry` | "Your {{app_name}} verification code" |
| `password-reset` | `reset_url`, `app_name`, `expiry` | "Reset your {{app_name}} password" |
| `magic-link` | `magic_url`, `app_name`, `expiry` | "Sign in to {{app_name}}" |
| `email-verify` | `verify_url`, `app_name` | "Verify your email for {{app_name}}" |
| `welcome` | `user_name`, `app_name` | "Welcome to {{app_name}}" |

**HTML style:** Minimal inline CSS, single-column, responsive, no branding. Each template has matching plain text fallback.

**Registration:**
```rust
// chorus-core/src/templates/mod.rs
pub fn builtin_templates() -> Vec<Template> { ... }
```

### Estimated LOC: ~300-370

---

## Phase 3c: chorus-mail + Integration Guide

### Problem

Email delivery requires paid third-party providers. No self-hosted option. No integration guide (#9).

### Solution

Docker image with Postfix + OpenDKIM. Bounce webhook. DNS tooling. Integration guide.

### Directory Structure

```
chorus-mail/
├── Dockerfile              # Alpine + Postfix + OpenDKIM + curl
├── config/
│   ├── main.cf             # Postfix main config
│   ├── master.cf           # Postfix services (includes bounce pipe)
│   └── opendkim.conf       # DKIM signing config
├── scripts/
│   ├── entrypoint.sh       # Startup: generate DKIM keys, configure domain
│   ├── dns-setup.sh        # Print DNS records (SPF, DKIM, DMARC, MX, A)
│   └── bounce-handler.sh   # Parse bounce → POST to chorus-server
└── README.md
```

### Environment Variables

```
MAIL_DOMAIN=example.com           # required
CHORUS_SERVER_URL=http://chorus-server:3000
BOUNCE_SECRET=<shared-secret>     # auth for bounce webhook
```

### Bounce Handling Flow

```
Remote server rejects email
  → Postfix generates bounce notification
    → master.cf pipe transport → bounce-handler.sh
      → Parse: recipient, reason, original message-id
        → curl POST ${CHORUS_SERVER_URL}/internal/bounces
            Header: X-Chorus-Secret: ${BOUNCE_SECRET}
            Body: { "message_id", "recipient", "reason" }
```

### chorus-server Changes

**New route:** `POST /internal/bounces`
- Validates `X-Chorus-Secret` header against `BOUNCE_SECRET` env var
- Updates message status → `failed`
- Inserts delivery event with reason
- ~50-60 LOC

**New route:** `GET /internal/dns-check?domain=example.com`
- DNS lookups for SPF, DKIM, DMARC, MX records
- Returns `{ "spf": bool, "dkim": bool, "dmarc": bool, "mx": bool }`
- Uses `hickory-resolver` crate
- ~80-100 LOC

**Config update:** Add `BOUNCE_SECRET` env var.

### DNS Setup CLI

```bash
$ docker exec chorus-mail sh /scripts/dns-setup.sh

Add these DNS records for: example.com

TXT  @                    "v=spf1 ip4:YOUR_SERVER_IP -all"
TXT  chorus._domainkey    "v=DKIM1; k=rsa; p=MIIBIjANBg..."
TXT  _dmarc               "v=DMARC1; p=quarantine; rua=mailto:postmaster@example.com"
MX   @                    mail.example.com (priority 10)
A    mail                 YOUR_SERVER_IP
```

### docker-compose.yml

```yaml
chorus-mail:
  build: ./chorus-mail
  environment:
    MAIL_DOMAIN: ${MAIL_DOMAIN}
    CHORUS_SERVER_URL: http://chorus-server:3000
    BOUNCE_SECRET: ${BOUNCE_SECRET}
  ports:
    - "25:25"
    - "587:587"
```

chorus-server uses chorus-mail via existing SMTP config:
```
SMTP_HOST=chorus-mail
SMTP_PORT=587
FROM_EMAIL=noreply@${MAIL_DOMAIN}
```

### Integration Guide

**File:** `docs/guides/auth-service-integration.md`

**Contents:**
1. Adapter pattern — wrap Chorus behind custom trait (Nucleus example)
2. Error mapping — ChorusError → app-specific errors
3. SMTP provider examples — SendGrid, Mailgun, Gmail config
4. chorus-mail setup — docker compose → DNS → verify → send
5. OTP routing — auto-detect email/phone
6. Test mode — `ch_test_` keys for development

### Estimated LOC: ~380-460 (Shell/Config/Rust/Docs)

---

## Effort Summary

| Phase | Scope | LOC | Issues |
|-------|-------|-----|--------|
| 3a | SMS check_status real API | ~120-150 | #8 |
| 3b | minijinja + 5 templates | ~300-370 | #10, #7 |
| 3c | chorus-mail + bounce + DNS + guide | ~380-460 | #9, close #6/#11 |
| **Total** | | **~800-980** | **All 6 resolved** |

Execution order: 3a → 3b → 3c (each phase independent, deployable separately).
