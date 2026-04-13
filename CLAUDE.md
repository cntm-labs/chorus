# CLAUDE.md — Chorus Project Guide

## Overview
Chorus is an open-source Communications Platform as a Service (CPaaS) built in Rust. Provides SMS and Email delivery with smart routing, multi-provider failover, and cost optimization.

## Tech Stack
- **Language:** Rust (stable) with Axum web framework
- **Database:** PostgreSQL 16 (message history, accounts, billing) + Redis 7 (async queue, caching)
- **Providers:** Telnyx, Twilio, Plivo (SMS) + Resend, AWS SES, Mailgun, SMTP (Email)
- **Payment:** Stripe + Stripe Tax
- **Monitoring:** Prometheus + Loki (data) → Strata (dashboard, separate repo)
- **SDKs:** Rust, TypeScript, Go, Java, Python, C

## Build Commands
```sh
cargo check --workspace          # Type check
cargo test --workspace           # Run all tests
cargo clippy --workspace -- -D warnings  # Lint
cargo fmt --all                  # Format
cargo deny check                 # License + advisory check
cargo llvm-cov nextest --workspace  # Test with coverage
```

## Project Structure
```
crates/                  # Publishable libraries (crates.io)
├── chorus-core          # Traits, routing engine, types, errors (leaf crate)
└── chorus-providers     # Telnyx, Twilio, Plivo, Resend, SES, Mailgun, SMTP, Mock adapters
services/                # Internal binaries (not published)
└── chorus-server        # Axum REST API, billing, dashboard
sdks/
├── rust/                # Native (uses chorus-core directly)
├── typescript/          # Node.js + Browser
├── go/
├── java/
├── python/
└── c/
```

## Dependency Rules (STRICT)
- chorus-core → external deps only (leaf crate, published to crates.io)
- chorus-providers → chorus-core + reqwest (published to crates.io)
- chorus-server → all crates (composition root, internal only, NOT published)
- SDKs → HTTP only (no Rust dependency except Rust SDK)

## Key Design Decisions
- **Waterfall routing:** OTP/notifications via email first (Resend, cheap/free) → SMS fallback only when no email — saves 60-80% cost
- **Async queue:** Accept request immediately (202) → process via Redis workers
- **Multi-provider failover:** If provider #1 fails → auto-retry with provider #2
- **Test mode:** `ch_test_` API keys log only, never send real messages
- **Self-hosted free:** MIT license, no billing when self-hosted

## Conventions
- All errors use ChorusError enum
- Provider credentials encrypted at rest (AES-GCM)
- E.164 format required for phone numbers
- Template variables use `{{variable}}` syntax
- Cost tracked in microdollars (BIGINT) to avoid float issues

## AI Development Guard Rails

### Anti-patterns to Avoid
- No `#[allow(dead_code)]` — use it or remove it
- No duplicate functions/structs — extract shared logic to chorus-core
- No spaghetti dependencies — follow dependency rules strictly
- No magic numbers — use named constants
- Max file ~300 lines — split if exceeding
- No `dbg!()`, `print!()`, `todo!()` in production code
- All public types/functions must have doc comments

### Design Patterns Used in Chorus

| Pattern | Where | Why |
|---------|-------|-----|
| Builder | `Chorus::builder()` | Complex config step-by-step |
| Strategy | `SmsSender`, `EmailSender` traits | Swap providers at runtime |
| Chain of Responsibility | `WaterfallRouter` | Try providers sequentially, fallback on failure |
| Facade | `Chorus` client | Hide router/template/provider complexity |
| Adapter | Provider implementations | Normalize different APIs to common trait |
| Template Method | `Template::render()` | Algorithm skeleton for variable replacement |
| Factory Method | `SesEmailSender::new()` | Create appropriate transport from config |

### Refactoring Rules
- **Extract Method** — if function > 30 lines, extract sub-functions
- **Replace Conditional with Polymorphism** — use trait dispatch over growing match chains
- **Introduce Parameter Object** — group related params into structs
- **Separate Query from Modifier** — read-only methods must not have side effects
