# crates.io Release Prep Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Prepare `chorus-core` and `chorus-providers` for publishing `0.1.0-beta` on crates.io with proper metadata and documentation.

**Architecture:** Update Cargo.toml metadata for both crates, add module-level and public API doc comments, create per-crate README files for crates.io display.

**Tech Stack:** Rust, Cargo, crates.io

**Execution order:** Metadata → chorus-core docs → chorus-providers docs → Per-crate READMEs → Verify → Dry-run publish

---

## Task 1: Update Crate Metadata

### Files
- Modify: `crates/chorus-core/Cargo.toml`
- Modify: `crates/chorus-providers/Cargo.toml`
- Modify: `.release-please-manifest.json`

### Step 1: Update chorus-core/Cargo.toml

Replace the `[package]` section:

```toml
[package]
name = "chorus-core"
version = "0.1.0-beta"
edition = "2021"
license = "MIT"
description = "Core traits and types for Chorus CPaaS — SMS, Email, OTP with smart routing and multi-provider failover"
repository = "https://github.com/cntm-labs/chorus"
homepage = "https://github.com/cntm-labs/chorus"
keywords = ["sms", "email", "otp", "cpaas", "messaging"]
categories = ["api-bindings", "network-programming"]
readme = "README.md"
rust-version = "1.85.0"
```

### Step 2: Update chorus-providers/Cargo.toml

Replace the `[package]` section:

```toml
[package]
name = "chorus-providers"
version = "0.1.0-beta"
edition = "2021"
license = "MIT"
description = "SMS and Email provider implementations for Chorus CPaaS — Telnyx, Twilio, Plivo, Resend, SES, SMTP"
repository = "https://github.com/cntm-labs/chorus"
homepage = "https://github.com/cntm-labs/chorus"
keywords = ["sms", "email", "twilio", "telnyx", "plivo"]
categories = ["api-bindings", "network-programming"]
readme = "README.md"
rust-version = "1.85.0"
```

Also update the chorus-core dependency version:

```toml
chorus-core = { path = "../chorus-core", version = "0.1.0-beta" }
```

### Step 3: Update .release-please-manifest.json

```json
{
  ".": "0.1.0-beta"
}
```

### Step 4: Verify compilation

Run: `cargo check --workspace`
Expected: Compiles with no errors

### Step 5: Commit

```bash
git add crates/chorus-core/Cargo.toml crates/chorus-providers/Cargo.toml .release-please-manifest.json
git commit -m "chore: update crate metadata for crates.io release (0.1.0-beta)"
```

---

## Task 2: Add chorus-core Documentation

### Files
- Modify: `crates/chorus-core/src/lib.rs`
- Modify: `crates/chorus-core/src/client.rs`
- Modify: `crates/chorus-core/src/sms.rs`
- Modify: `crates/chorus-core/src/email.rs`
- Modify: `crates/chorus-core/src/router.rs`
- Modify: `crates/chorus-core/src/template.rs`
- Modify: `crates/chorus-core/src/error.rs`
- Modify: `crates/chorus-core/src/types.rs`

### Step 1: Add module docs to lib.rs

Replace the entire `crates/chorus-core/src/lib.rs` with:

```rust
//! # chorus-core
//!
//! Core traits, types, and routing engine for Chorus CPaaS.
//!
//! Chorus provides SMS and Email delivery with smart routing, multi-provider
//! failover, and cost optimization through waterfall routing (email-first,
//! SMS-fallback).
//!
//! ## Key Components
//!
//! - [`Chorus`](client::Chorus) — Main client with builder pattern
//! - [`SmsSender`](sms::SmsSender) / [`EmailSender`](email::EmailSender) — Provider traits
//! - [`WaterfallRouter`](router::WaterfallRouter) — Cost-optimized routing engine
//! - [`Template`](template::Template) — `{{variable}}` template rendering
//!
//! ## Quick Start
//!
//! ```rust,no_run
//! use chorus::client::Chorus;
//! use chorus::types::SmsMessage;
//! use std::sync::Arc;
//!
//! # async fn example() -> Result<(), chorus::error::ChorusError> {
//! let chorus = Chorus::builder()
//!     // .add_sms_provider(Arc::new(my_provider))
//!     .default_from_sms("+1234567890".into())
//!     .build();
//!
//! let msg = SmsMessage {
//!     to: "+0987654321".into(),
//!     body: "Hello from Chorus!".into(),
//!     from: None,
//! };
//! let result = chorus.send_sms(&msg).await?;
//! # Ok(())
//! # }
//! ```

pub mod client;
pub mod email;
pub mod error;
pub mod router;
pub mod sms;
pub mod template;
pub mod types;
```

### Step 2: Add doc comments to client.rs public items

Add `///` doc comments to each public method on `Chorus` and `ChorusBuilder`. Only add docs — do not change any logic. The struct `Chorus` already has a doc comment.

```rust
/// Creates a new [`ChorusBuilder`] to configure the client.
pub fn builder() -> ChorusBuilder {

/// Sends an SMS message, applying `default_from_sms` if the message has no `from`.
pub async fn send_sms(&self, msg: &SmsMessage) -> Result<SendResult, ChorusError> {

/// Sends an email message directly through the router.
pub async fn send_email(&self, msg: &EmailMessage) -> Result<SendResult, ChorusError> {

/// Renders a template by slug and sends the result as an email.
pub async fn send_email_template(

/// Sends a one-time password via waterfall routing (email for `@` recipients, SMS for phone numbers).
pub async fn send_otp(
```

Add to `ChorusBuilder`:

```rust
/// Builder for configuring a [`Chorus`] client.
pub struct ChorusBuilder {

/// Adds an SMS provider to the routing chain.
pub fn add_sms_provider(

/// Adds an email provider to the routing chain.
pub fn add_email_provider(

/// Registers an email template for use with [`Chorus::send_email_template`].
pub fn add_template(

/// Sets the default `from` address for emails sent via templates.
pub fn default_from_email(

/// Sets the default `from` number for SMS messages without an explicit sender.
pub fn default_from_sms(

/// Builds the [`Chorus`] client with the configured providers and templates.
pub fn build(self) -> Chorus {
```

### Step 3: Add doc comments to sms.rs

```rust
/// Trait for SMS delivery providers.
///
/// Implement this trait to add a new SMS provider to Chorus.
/// Providers are used by [`WaterfallRouter`](crate::router::WaterfallRouter) for
/// multi-provider failover.
#[async_trait]
pub trait SmsSender: Send + Sync {
    /// Returns the provider name (e.g., `"twilio"`, `"telnyx"`).
    fn provider_name(&self) -> &str;
    /// Sends an SMS message and returns the delivery result.
    async fn send(&self, msg: &SmsMessage) -> Result<SendResult, ChorusError>;
    /// Checks the delivery status of a previously sent message by ID.
    async fn check_status(&self, message_id: &str) -> Result<DeliveryStatus, ChorusError>;
}
```

### Step 4: Add doc comments to email.rs

```rust
/// Trait for email delivery providers.
///
/// Implement this trait to add a new email provider to Chorus.
/// Providers are used by [`WaterfallRouter`](crate::router::WaterfallRouter) for
/// multi-provider failover.
#[async_trait]
pub trait EmailSender: Send + Sync {
    /// Returns the provider name (e.g., `"resend"`, `"ses"`).
    fn provider_name(&self) -> &str;
    /// Sends an email message and returns the delivery result.
    async fn send(&self, msg: &EmailMessage) -> Result<SendResult, ChorusError>;
}
```

### Step 5: Add doc comments to router.rs

```rust
/// Waterfall router: tries each provider in order, falls back to the next on failure.
///
/// Optimizes cost by trying cheaper channels (email) before expensive ones (SMS).
/// For OTP delivery, automatically detects whether the recipient is an email address
/// or phone number and routes accordingly.
pub struct WaterfallRouter {
```

Add to public methods:

```rust
/// Creates an empty router with no providers.
pub fn new() -> Self {

/// Adds an SMS provider to the routing chain. Providers are tried in insertion order.
pub fn add_sms(

/// Adds an email provider to the routing chain. Providers are tried in insertion order.
pub fn add_email(

/// Sends an OTP via waterfall routing. Routes to email if recipient contains `@`, otherwise SMS.
pub async fn send_otp(

/// Sends an SMS directly, trying each SMS provider in order until one succeeds.
pub async fn send_sms(

/// Sends an email directly, trying each email provider in order until one succeeds.
pub async fn send_email(
```

### Step 6: Add doc comments to template.rs

```rust
/// An email template with `{{variable}}` placeholders.
pub struct Template {

/// The result of rendering a [`Template`] with variable values.
pub struct RenderedTemplate {

/// Renders the template by replacing `{{variable}}` placeholders with provided values.
/// Variables not found in the map are left as-is.
pub fn render(
```

### Step 7: Add doc comments to error.rs

```rust
/// Errors that can occur during Chorus operations.
#[derive(Debug, Error)]
pub enum ChorusError {
    /// A specific provider returned an error.
    #[error("provider error ({provider}): {message}")]
    Provider { provider: String, message: String },

    /// All configured providers failed to deliver the message.
    #[error("all providers failed")]
    AllProvidersFailed,

    /// Input validation failed (e.g., missing required field).
    #[error("validation error: {0}")]
    Validation(String),

    /// The requested template slug was not found.
    #[error("template not found: {0}")]
    TemplateNotFound(String),

    /// Account quota has been exceeded.
    #[error("quota exceeded: {0}")]
    QuotaExceeded(String),

    /// The provided API key is invalid.
    #[error("invalid api key")]
    InvalidApiKey,

    /// Request was rate limited. Retry after the specified duration.
    #[error("rate limited")]
    RateLimited { retry_after_secs: u64 },

    /// An unexpected internal error occurred.
    #[error("internal error: {0}")]
    Internal(#[from] anyhow::Error),
}
```

### Step 8: Add doc comments to types.rs

```rust
/// An SMS message to be sent to a single recipient.
pub struct SmsMessage {

/// An email message with subject, HTML body, and plain text fallback.
pub struct EmailMessage {

/// An email message that references a template by slug instead of inline content.
pub struct TemplateEmailMessage {

/// The result of a successful message delivery attempt.
pub struct SendResult {

/// The communication channel used for delivery.
pub enum Channel {

/// The delivery status of a sent message.
pub enum DeliveryStatus {
```

### Step 9: Verify docs build

Run: `cargo doc --workspace --no-deps 2>&1`
Expected: No warnings about missing docs

### Step 10: Commit

```bash
git add crates/chorus-core/src/
git commit -m "docs: add doc comments to all public types and methods in chorus-core"
```

---

## Task 3: Add chorus-providers Documentation

### Files
- Modify: `crates/chorus-providers/src/lib.rs`

### Step 1: Add module docs to lib.rs

Replace `crates/chorus-providers/src/lib.rs` with:

```rust
//! # chorus-providers
//!
//! SMS and Email provider implementations for Chorus CPaaS.
//!
//! ## SMS Providers
//!
//! | Provider | Struct | API |
//! |----------|--------|-----|
//! | Telnyx | [`sms::telnyx::TelnyxSmsSender`] | REST |
//! | Twilio | [`sms::twilio::TwilioSmsSender`] | REST |
//! | Plivo | [`sms::plivo::PlivoSmsSender`] | REST |
//! | Mock | [`sms::mock::MockSmsSender`] | In-memory (testing) |
//!
//! ## Email Providers
//!
//! | Provider | Struct | API |
//! |----------|--------|-----|
//! | Resend | [`email::resend::ResendEmailSender`] | REST |
//! | AWS SES | [`email::ses::SesEmailSender`] | SMTP |
//! | SMTP | [`email::smtp::SmtpEmailSender`] | SMTP |
//! | Mock | [`email::mock::MockEmailSender`] | In-memory (testing) |
//!
//! All providers implement [`chorus::sms::SmsSender`] or
//! [`chorus::email::EmailSender`] and can be used interchangeably
//! with [`chorus::router::WaterfallRouter`].

pub mod email;
pub mod sms;
```

### Step 2: Verify docs build

Run: `cargo doc --workspace --no-deps 2>&1`
Expected: No errors

### Step 3: Commit

```bash
git add crates/chorus-providers/src/lib.rs
git commit -m "docs: add module-level documentation to chorus-providers"
```

---

## Task 4: Create Per-Crate README Files

### Files
- Create: `crates/chorus-core/README.md`
- Create: `crates/chorus-providers/README.md`

### Step 1: Create chorus-core/README.md

```markdown
# chorus-core

Core traits, types, and routing engine for [Chorus](https://github.com/cntm-labs/chorus) CPaaS.

## Features

- **Waterfall routing** — Email-first, SMS-fallback for cost optimization
- **Multi-provider failover** — Auto-retry with next provider on failure
- **Template engine** — `{{variable}}` syntax with rendering
- **Builder pattern** — Fluent API for client configuration

## Usage

```rust
use chorus::client::Chorus;
use chorus::types::SmsMessage;

let chorus = Chorus::builder()
    .add_sms_provider(my_provider)
    .default_from_sms("+1234567890".into())
    .build();

let msg = SmsMessage {
    to: "+0987654321".into(),
    body: "Hello from Chorus!".into(),
    from: None,
};
let result = chorus.send_sms(&msg).await?;
```

See the [main repository](https://github.com/cntm-labs/chorus) for full documentation.

## License

MIT
```

### Step 2: Create chorus-providers/README.md

```markdown
# chorus-providers

SMS and Email provider implementations for [Chorus](https://github.com/cntm-labs/chorus) CPaaS.

## Supported Providers

### SMS
- **Telnyx** — `TelnyxSmsSender`
- **Twilio** — `TwilioSmsSender`
- **Plivo** — `PlivoSmsSender`
- **Mock** — `MockSmsSender` (for testing)

### Email
- **Resend** — `ResendEmailSender`
- **AWS SES** — `SesEmailSender`
- **SMTP** — `SmtpEmailSender`
- **Mock** — `MockEmailSender` (for testing)

## Usage

```rust
use chorus_providers::sms::telnyx::TelnyxSmsSender;
use chorus_providers::email::resend::ResendEmailSender;

let sms = TelnyxSmsSender::new("api_key".into(), Some("+1234567890".into()));
let email = ResendEmailSender::new("api_key".into(), "noreply@example.com".into());
```

All providers implement `chorus::sms::SmsSender` or `chorus::email::EmailSender`.

See the [main repository](https://github.com/cntm-labs/chorus) for full documentation.

## License

MIT
```

### Step 3: Commit

```bash
git add crates/chorus-core/README.md crates/chorus-providers/README.md
git commit -m "docs: add per-crate README files for crates.io"
```

---

## Task 5: Final Verification & Dry-Run Publish

### Step 1: Run full check suite

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo doc --workspace --no-deps
```

All must pass with zero warnings/errors.

### Step 2: Dry-run publish chorus-core

```bash
cargo publish --dry-run -p chorus-core
```

Expected: No errors. This validates metadata, README, and package contents without actually publishing.

### Step 3: Dry-run publish chorus-providers

```bash
cargo publish --dry-run -p chorus-providers
```

Expected: No errors.

### Step 4: Commit any remaining fixes

```bash
git status
# If changes needed: fix and commit
```

---

## Verification Checklist

After all tasks complete:

```bash
# Full test suite
cargo test --workspace

# Docs build
cargo doc --workspace --no-deps

# Lint
cargo clippy --workspace --all-targets -- -D warnings

# Dry-run publish
cargo publish --dry-run -p chorus-core
cargo publish --dry-run -p chorus-providers
```

All must pass before actual publish.
