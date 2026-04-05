<div align="center">

# chorus

**Open-source Communications Platform as a Service (CPaaS) — SMS, Email, OTP with smart routing and multi-provider failover.**

[![CI](https://github.com/cntm-labs/chorus/actions/workflows/ci.yml/badge.svg)](https://github.com/cntm-labs/chorus/actions/workflows/ci.yml)
[![Security](https://github.com/cntm-labs/chorus/actions/workflows/security.yml/badge.svg)](https://github.com/cntm-labs/chorus/actions/workflows/security.yml)
[![Release](https://github.com/cntm-labs/chorus/actions/workflows/release-please.yml/badge.svg)](https://github.com/cntm-labs/chorus/actions/workflows/release-please.yml)
[![codecov](https://codecov.io/gh/cntm-labs/chorus/branch/main/graph/badge.svg)](https://codecov.io/gh/cntm-labs/chorus)

[![crates.io chorus-core](https://img.shields.io/crates/v/chorus-core?label=chorus-core&color=fc8d62)](https://crates.io/crates/chorus-core)
[![crates.io chorus-providers](https://img.shields.io/crates/v/chorus-providers?label=chorus-providers&color=fc8d62)](https://crates.io/crates/chorus-providers)
[![docs.rs](https://img.shields.io/docsrs/chorus-core?label=docs.rs)](https://docs.rs/chorus-core)

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

Rust backend (Axum) with waterfall routing — email first (free), SMS fallback (paid) — saving 60-80% cost. Supports 6 providers with automatic failover.

## Features

- **Waterfall routing** — email first (free) then SMS fallback (paid), saving 60-80% cost
- **Multi-provider failover** — auto-retry with next provider on failure
- **6 providers** — Telnyx, Twilio, Plivo (SMS) + Resend, AWS SES, SMTP (Email)
- **Template engine** — `{{variable}}` syntax with OTP generation
- **Test mode** — `ch_test_` API keys log only, never send real messages
- **Self-hosted free** — MIT license, no billing when self-hosted

## Quick Start

```rust
use chorus_core::client::Chorus;
use chorus_core::types::SmsMessage;
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
