# CLAUDE.md — Chorus Project Guide

## Overview
Chorus is an open-source Communications Platform as a Service (CPaaS) built in Rust. Provides SMS and Email delivery with smart routing, multi-provider failover, and cost optimization.

## Tech Stack
- **Language:** Rust (stable) with Axum web framework
- **Database:** PostgreSQL 16 (message history, accounts, billing) + Redis 7 (async queue, caching)
- **Providers:** Telnyx, Twilio, Plivo (SMS) + Resend, AWS SES, SMTP (Email)
- **Payment:** Stripe + Stripe Tax
- **Monitoring:** Prometheus + Loki (data) → Strata (dashboard, separate repo)
- **SDKs:** Rust, TypeScript, Go, Java, Python, C

## Build Commands
```sh
cargo check --workspace          # Type check
cargo test --workspace           # Run all tests
cargo clippy --workspace -- -D warnings  # Lint
cargo fmt --all                  # Format
```

## Project Structure
```
crates/
├── chorus-core        # Traits, routing engine, types, errors (leaf crate)
├── chorus-providers   # Telnyx, Twilio, Plivo, Resend, SES, SMTP, Mock adapters
└── chorus-server      # Axum REST API, billing, dashboard
sdks/
├── rust/              # Native (uses chorus-core directly)
├── typescript/        # Node.js + Browser
├── go/
├── java/
├── python/
└── c/
```

## Dependency Rules (STRICT)
- chorus-core → external deps only (leaf crate)
- chorus-providers → chorus-core + reqwest
- chorus-server → all crates (composition root)
- SDKs → HTTP only (no Rust dependency except Rust SDK)

## Key Design Decisions
- **Waterfall routing:** Email first (free) → SMS fallback (paid) — saves 60-80% cost
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
