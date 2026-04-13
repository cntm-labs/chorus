<div align="center">

# chorus

**Open-source Communications Platform as a Service (CPaaS) — SMS, Email, OTP with smart routing and multi-provider failover.**

[![CI](https://github.com/cntm-labs/chorus/actions/workflows/ci.yml/badge.svg)](https://github.com/cntm-labs/chorus/actions/workflows/ci.yml)
[![Security](https://github.com/cntm-labs/chorus/actions/workflows/security.yml/badge.svg)](https://github.com/cntm-labs/chorus/actions/workflows/security.yml)
[![Release](https://github.com/cntm-labs/chorus/actions/workflows/release-please.yml/badge.svg)](https://github.com/cntm-labs/chorus/actions/workflows/release-please.yml)
[![codecov](https://codecov.io/gh/cntm-labs/chorus/branch/main/graph/badge.svg)](https://codecov.io/gh/cntm-labs/chorus)

[![crates.io chorus-rs](https://img.shields.io/crates/v/chorus-rs?label=chorus-rs&color=fc8d62)](https://crates.io/crates/chorus-rs)
[![crates.io chorus-core](https://img.shields.io/crates/v/chorus-core?label=chorus-core&color=fc8d62)](https://crates.io/crates/chorus-core)
[![crates.io chorus-providers](https://img.shields.io/crates/v/chorus-providers?label=chorus-providers&color=fc8d62)](https://crates.io/crates/chorus-providers)
[![npm @cntm-labs/chorus](https://img.shields.io/npm/v/@cntm-labs/chorus?label=@cntm-labs/chorus&color=cb3837)](https://www.npmjs.com/package/@cntm-labs/chorus)
[![Maven Central](https://img.shields.io/maven-central/v/com.cntm-labs/chorus?label=com.cntm-labs:chorus&color=C71A36)](https://central.sonatype.com/artifact/com.cntm-labs/chorus)
[![docs.rs](https://img.shields.io/docsrs/chorus-rs?label=docs.rs)](https://docs.rs/chorus-rs)

[![Rust](https://img.shields.io/badge/Rust-2k_LOC-dea584?logo=rust&logoColor=white)](crates/)
[![Total Lines](https://img.shields.io/badge/Total-2k+_LOC-blue)](./)

[![Rust](https://img.shields.io/badge/Rust-dea584?logo=rust&logoColor=white)](https://www.rust-lang.org/)
[![Axum](https://img.shields.io/badge/Axum-dea584?logo=rust&logoColor=white)](https://github.com/tokio-rs/axum)
[![PostgreSQL](https://img.shields.io/badge/PostgreSQL-4169E1?logo=postgresql&logoColor=white)](https://www.postgresql.org/)
[![Redis](https://img.shields.io/badge/Redis-DC382D?logo=redis&logoColor=white)](https://redis.io/)
[![Docker](https://img.shields.io/badge/Docker-2496ED?logo=docker&logoColor=white)](https://www.docker.com/)
[![Stripe](https://img.shields.io/badge/Stripe-635BFF?logo=stripe&logoColor=white)](https://stripe.com/)

</div>

---

Rust backend (Axum) with waterfall routing — OTP and notifications via email first (cheap/free), SMS fallback only when needed (paid) — saving 60-80% cost. Supports 7 providers with automatic failover.

## Features

- **Waterfall routing** — send OTP/notifications via email first (Resend, free tier), fall back to SMS only when no email available — saving 60-80% cost
- **Multi-provider failover** — auto-retry with next provider on failure
- **7 providers** — Telnyx, Twilio, Plivo (SMS) + Resend, AWS SES, Mailgun, SMTP (Email)
- **Batch send** — send SMS or Email to multiple recipients in one call
- **Webhooks** — real-time delivery status notifications with HMAC-SHA256 signatures
- **Template engine** — `{{variable}}` syntax with OTP generation
- **SDKs** — Rust, TypeScript, Java (Go, Python, C coming soon)
- **Test mode** — `ch_test_` API keys log only, never send real messages
- **Self-hosted free** — MIT license, no billing when self-hosted

## Quick Start

```rust
use chorus::client::Chorus;
use chorus::types::SmsMessage;
use std::sync::Arc;

let chorus = Chorus::builder()
    .add_sms_provider(Arc::new(telnyx))
    .add_email_provider(Arc::new(resend))
    .default_from_sms("+1234567890".into())
    .build();

// Send SMS
let msg = SmsMessage {
    to: "+0987654321".into(),
    body: "Hello from Chorus!".into(),
    from: None,
};
chorus.send_sms(&msg).await?;

// Send OTP via email with SMS fallback
chorus.send_otp("user@example.com", "123456", "MyApp").await?;
```

## Architecture

```
crates/                  # Publishable libraries
├── chorus-core          # Traits, routing engine, types, errors (leaf crate)
└── chorus-providers     # Telnyx, Twilio, Plivo, Resend, SES, Mailgun, SMTP adapters
services/                # Internal binaries (not published)
└── chorus-server        # Axum REST API, billing, dashboard
sdks/
├── rust/                # Native SDK (chorus-rs, re-exports chorus-core + providers)
├── typescript/          # Node.js + Browser (@cntm-labs/chorus)
├── java/                # Java 11+ (com.cntm-labs:chorus)
├── go/                  # Coming soon
├── python/              # Coming soon
└── c/                   # Coming soon
```

## Development

```sh
cargo check --workspace          # Type check
cargo test --workspace           # Run all tests
cargo clippy --workspace -- -D warnings  # Lint
cargo fmt --all                  # Format

# Setup pre-commit hook
git config core.hooksPath .githooks
```

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for guidelines.

## License

[MIT](LICENSE)
