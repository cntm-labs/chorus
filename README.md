# Chorus

[![CI](https://github.com/cntm-labs/chorus/actions/workflows/ci.yml/badge.svg)](https://github.com/cntm-labs/chorus/actions/workflows/ci.yml)
[![Security](https://github.com/cntm-labs/chorus/actions/workflows/security.yml/badge.svg)](https://github.com/cntm-labs/chorus/actions/workflows/security.yml)
[![codecov](https://codecov.io/gh/cntm-labs/chorus/branch/main/graph/badge.svg)](https://codecov.io/gh/cntm-labs/chorus)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/Rust-stable-orange.svg)](https://www.rust-lang.org/)
[![Lines of Code](https://img.shields.io/badge/Lines_of_Code-1.5k-informational.svg)]()

Open-source Communications Platform as a Service (CPaaS) — SMS, Email, OTP delivery with smart routing, multi-provider failover, and cost optimization. Built in Rust.

## Features

- **Waterfall Routing** — Email first (free) then SMS fallback (paid), saving 60-80% cost
- **Multi-Provider Failover** — Auto-retry with next provider on failure
- **6 Providers** — Telnyx, Twilio, Plivo (SMS) + Resend, AWS SES, SMTP (Email)
- **Template Engine** — `{{variable}}` syntax with OTP generation
- **Test Mode** — `ch_test_` API keys log only, never send real messages
- **Self-Hosted Free** — MIT license, no billing when self-hosted

## Quick Start

```rust
use chorus_core::Chorus;

let chorus = Chorus::builder()
    .with_sms_provider(telnyx)
    .with_email_provider(resend)
    .enable_waterfall_routing()
    .build();

// Send SMS
chorus.send_sms("+1234567890", "Hello from Chorus!").await?;

// Send OTP via email with SMS fallback
chorus.send_otp("user@example.com", 6).await?;
```

## Architecture

```
crates/
├── chorus-core        # Traits, routing engine, types, errors (leaf crate)
├── chorus-providers   # Telnyx, Twilio, Plivo, Resend, SES, SMTP adapters
└── chorus-server      # Axum REST API, billing, dashboard (coming soon)
sdks/
├── rust/              # Native SDK
├── typescript/        # Node.js + Browser
├── go/
├── java/
├── python/
└── c/
```

## Tech Stack

| Component | Technology |
|-----------|-----------|
| Language | Rust (stable) |
| Web Framework | Axum |
| Database | PostgreSQL 16 + Redis 7 |
| SMS Providers | Telnyx, Twilio, Plivo |
| Email Providers | Resend, AWS SES, SMTP |
| Payment | Stripe + Stripe Tax |
| Monitoring | Prometheus + Loki |
| CI/CD | GitHub Actions + Release Please |
| Container | Docker (ghcr.io) |

## Development

```sh
# Prerequisites: Rust stable toolchain
cargo check --workspace          # Type check
cargo test --workspace           # Run all tests
cargo clippy --workspace -- -D warnings  # Lint
cargo fmt --all                  # Format
cargo deny check                 # License + advisory check

# Dev watch (requires bacon)
bacon                            # Auto-check on save
bacon clippy                     # Auto-lint on save
bacon test                       # Auto-test on save

# Setup pre-commit hook
./scripts/setup-hooks.sh
```

## Contributing

1. Fork the repository
2. Create a feature branch (`git checkout -b feature/amazing-feature`)
3. Run `./scripts/setup-hooks.sh` to install pre-commit hooks
4. Make your changes and ensure all checks pass
5. Commit with [Conventional Commits](https://www.conventionalcommits.org/) format
6. Push and open a Pull Request

## License

[MIT](LICENSE)
