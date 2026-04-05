# Phase 2a: crates.io Release Prep Design

**Goal:** Prepare `chorus-core` and `chorus-providers` for publishing `0.1.0-beta` on crates.io.

**Crate names:** `chorus-core`, `chorus-providers` (verified available on crates.io)

## 1. Crate Metadata

Add required/recommended fields to both crate Cargo.toml files:

```toml
version = "0.1.0-beta"
repository = "https://github.com/cntm-labs/chorus"
homepage = "https://github.com/cntm-labs/chorus"
keywords = ["sms", "email", "otp", "cpaas", "messaging"]
categories = ["api-bindings", "network-programming"]
readme = "README.md"
rust-version = "1.85.0"
```

Update `.release-please-manifest.json` to `0.1.0-beta`.

## 2. Documentation

### Module-level docs (`//!` in lib.rs)
- `chorus-core`: What the crate is, core concepts (traits, router, template), minimal usage example
- `chorus-providers`: What providers are available, how to construct them

### Public API docs (`///` on types/methods)
- Key types: `Chorus`, `ChorusBuilder`, `SmsSender`, `EmailSender`, `WaterfallRouter`, `Template`, `ChorusError`
- Key methods: `builder()`, `send_sms()`, `send_email()`, `send_otp()`, `render()`
- Keep concise — one-liner + parameters where needed

### Per-crate README.md
- `crates/chorus-core/README.md` — shown on crates.io page
- `crates/chorus-providers/README.md` — shown on crates.io page

## 3. Out of Scope (YAGNI)

- Feature flags
- CHANGELOG.md (Release Please generates)
- rustdoc tests
- `#[must_use]` attributes
- SDKs, server, Docker
