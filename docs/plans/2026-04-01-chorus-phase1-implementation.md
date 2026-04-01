# Chorus Phase 1 — Core Library Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Build `chorus-core` and `chorus-providers` crates so Nucleus can import them directly as a Rust library for SMS and Email delivery with waterfall routing.

**Architecture:** Workspace with 2 crates — `chorus-core` (traits, types, routing engine) and `chorus-providers` (Telnyx, Twilio, Plivo, Resend, SES, SMTP, Mock implementations). Both are libraries, no server yet.

**Tech Stack:** Rust (stable), async-trait, reqwest (HTTP client), serde/serde_json, tokio, thiserror, anyhow, tracing, uuid, chrono, base64, ring (CSPRNG), aes-gcm (encryption)

**Execution order:** Project scaffold → Types/Errors → Traits → Mock providers → Waterfall router → Real SMS providers → Real Email providers → Integration test → OTP module

---

## Task 1: Project Scaffold — Cargo Workspace

### Files
- Modify: `Cargo.toml` — workspace definition
- Create: `crates/chorus-core/Cargo.toml`
- Create: `crates/chorus-core/src/lib.rs`
- Create: `crates/chorus-providers/Cargo.toml`
- Create: `crates/chorus-providers/src/lib.rs`
- Modify: `CLAUDE.md` — verify project structure
- Create: `.env.example`
- Create: `LICENSE`

### Step 1: Create workspace Cargo.toml

```toml
# Cargo.toml (workspace root)
[workspace]
resolver = "2"
members = [
    "crates/chorus-core",
    "crates/chorus-providers",
]

[workspace.dependencies]
# Async
tokio = { version = "1", features = ["full"] }
async-trait = "0.1"

# Serialization
serde = { version = "1", features = ["derive"] }
serde_json = "1"

# HTTP
reqwest = { version = "0.12", features = ["json"] }

# Error handling
thiserror = "2"
anyhow = "1"

# Observability
tracing = "0.1"

# Crypto
ring = "0.17"
aes-gcm = "0.10"
base64 = "0.22"
hex = "0.4"

# Utilities
uuid = { version = "1", features = ["v4", "serde"] }
chrono = { version = "0.4", features = ["serde"] }
rand = "0.8"
url = "2"

[workspace.lints.rust]
unsafe_code = "forbid"

[workspace.lints.clippy]
all = { level = "warn", priority = -1 }
module_name_repetitions = "allow"
must_use_candidate = "allow"
missing_errors_doc = "allow"
missing_panics_doc = "allow"
```

### Step 2: Create chorus-core crate

```toml
# crates/chorus-core/Cargo.toml
[package]
name = "chorus-core"
version = "0.1.0"
edition = "2021"
license = "MIT"
description = "Core traits and types for Chorus CPaaS"

[dependencies]
serde = { workspace = true }
serde_json = { workspace = true }
thiserror = { workspace = true }
anyhow = { workspace = true }
async-trait = { workspace = true }
tracing = { workspace = true }
uuid = { workspace = true }
chrono = { workspace = true }

[lints]
workspace = true
```

```rust
// crates/chorus-core/src/lib.rs
pub mod error;
pub mod types;
```

### Step 3: Create chorus-providers crate

```toml
# crates/chorus-providers/Cargo.toml
[package]
name = "chorus-providers"
version = "0.1.0"
edition = "2021"
license = "MIT"
description = "SMS and Email provider implementations for Chorus CPaaS"

[dependencies]
chorus-core = { path = "../chorus-core", version = "0.1.0" }
reqwest = { workspace = true }
serde = { workspace = true }
serde_json = { workspace = true }
async-trait = { workspace = true }
anyhow = { workspace = true }
tracing = { workspace = true }
tokio = { workspace = true }

[lints]
workspace = true
```

```rust
// crates/chorus-providers/src/lib.rs
pub mod sms;
pub mod email;
```

### Step 4: Create LICENSE (MIT) and .env.example

```
# .env.example
# SMS Providers (configure at least one)
# TELNYX_API_KEY=KEY_xxx
# TWILIO_ACCOUNT_SID=ACxxx
# TWILIO_AUTH_TOKEN=xxx
# TWILIO_FROM_NUMBER=+1xxx
# PLIVO_AUTH_ID=xxx
# PLIVO_AUTH_TOKEN=xxx
# PLIVO_FROM_NUMBER=+1xxx

# Email Providers (configure at least one)
# RESEND_API_KEY=re_xxx
# AWS_SES_ACCESS_KEY=xxx
# AWS_SES_SECRET_KEY=xxx
# AWS_SES_REGION=us-east-1
# SMTP_HOST=smtp.gmail.com
# SMTP_PORT=587
# SMTP_USERNAME=xxx
# SMTP_PASSWORD=xxx

# General
# FROM_EMAIL=noreply@yourdomain.com
# FROM_NAME=YourApp
```

### Step 5: Verify workspace compiles

Run: `cargo check --workspace`
Expected: Compiles with no errors

### Step 6: Commit

```bash
git add -A
git commit -m "chore: scaffold Cargo workspace with chorus-core and chorus-providers"
```

---

## Task 2: Types and Errors — chorus-core

### Files
- Create: `crates/chorus-core/src/types.rs`
- Create: `crates/chorus-core/src/error.rs`
- Modify: `crates/chorus-core/src/lib.rs`

### Step 1: Write error types

```rust
// crates/chorus-core/src/error.rs
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ChorusError {
    #[error("provider error ({provider}): {message}")]
    Provider { provider: String, message: String },

    #[error("all providers failed")]
    AllProvidersFailed,

    #[error("validation error: {0}")]
    Validation(String),

    #[error("template not found: {0}")]
    TemplateNotFound(String),

    #[error("quota exceeded: {0}")]
    QuotaExceeded(String),

    #[error("invalid api key")]
    InvalidApiKey,

    #[error("rate limited")]
    RateLimited { retry_after_secs: u64 },

    #[error("internal error: {0}")]
    Internal(#[from] anyhow::Error),
}
```

### Step 2: Write core types

```rust
// crates/chorus-core/src/types.rs
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SmsMessage {
    pub to: String,
    pub body: String,
    pub from: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmailMessage {
    pub to: String,
    pub subject: String,
    pub html_body: String,
    pub text_body: String,
    pub from: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemplateEmailMessage {
    pub to: String,
    pub template_slug: String,
    pub variables: serde_json::Value,
    pub from: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SendResult {
    pub message_id: String,
    pub provider: String,
    pub channel: Channel,
    pub status: DeliveryStatus,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Channel {
    Sms,
    Email,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryStatus {
    Queued,
    Sent,
    Delivered,
    Failed { reason: String },
}

impl std::fmt::Display for Channel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Channel::Sms => write!(f, "sms"),
            Channel::Email => write!(f, "email"),
        }
    }
}
```

### Step 3: Update lib.rs

```rust
// crates/chorus-core/src/lib.rs
pub mod error;
pub mod types;
```

### Step 4: Write tests for types

```rust
// Add to bottom of crates/chorus-core/src/types.rs
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channel_display() {
        assert_eq!(Channel::Sms.to_string(), "sms");
        assert_eq!(Channel::Email.to_string(), "email");
    }

    #[test]
    fn sms_message_serializes() {
        let msg = SmsMessage {
            to: "+66812345678".to_string(),
            body: "Hello".to_string(),
            from: None,
        };
        let json = serde_json::to_value(&msg).unwrap();
        assert_eq!(json["to"], "+66812345678");
    }

    #[test]
    fn delivery_status_serializes() {
        let sent = DeliveryStatus::Sent;
        let json = serde_json::to_string(&sent).unwrap();
        assert_eq!(json, "\"sent\"");

        let failed = DeliveryStatus::Failed { reason: "timeout".to_string() };
        let json = serde_json::to_value(&failed).unwrap();
        assert_eq!(json["reason"], "timeout");
    }
}
```

### Step 5: Verify tests pass

Run: `cargo test --workspace`

### Step 6: Commit

```bash
git commit -m "feat(core): add ChorusError, types (SmsMessage, EmailMessage, SendResult, Channel, DeliveryStatus)"
```

---

## Task 3: SMS and Email Traits — chorus-core

### Files
- Create: `crates/chorus-core/src/sms.rs`
- Create: `crates/chorus-core/src/email.rs`
- Modify: `crates/chorus-core/src/lib.rs`

### Step 1: Define SmsSender trait

```rust
// crates/chorus-core/src/sms.rs
use async_trait::async_trait;
use crate::error::ChorusError;
use crate::types::{DeliveryStatus, SendResult, SmsMessage};

#[async_trait]
pub trait SmsSender: Send + Sync {
    fn provider_name(&self) -> &str;
    async fn send(&self, msg: &SmsMessage) -> Result<SendResult, ChorusError>;
    async fn check_status(&self, message_id: &str) -> Result<DeliveryStatus, ChorusError>;
}
```

### Step 2: Define EmailSender trait

```rust
// crates/chorus-core/src/email.rs
use async_trait::async_trait;
use crate::error::ChorusError;
use crate::types::{EmailMessage, SendResult};

#[async_trait]
pub trait EmailSender: Send + Sync {
    fn provider_name(&self) -> &str;
    async fn send(&self, msg: &EmailMessage) -> Result<SendResult, ChorusError>;
}
```

### Step 3: Update lib.rs

```rust
// crates/chorus-core/src/lib.rs
pub mod email;
pub mod error;
pub mod sms;
pub mod types;
```

### Step 4: Verify compiles

Run: `cargo check --workspace`

### Step 5: Commit

```bash
git commit -m "feat(core): add SmsSender and EmailSender traits"
```

---

## Task 4: Mock Providers — chorus-providers

### Files
- Create: `crates/chorus-providers/src/sms/mod.rs`
- Create: `crates/chorus-providers/src/sms/mock.rs`
- Create: `crates/chorus-providers/src/email/mod.rs`
- Create: `crates/chorus-providers/src/email/mock.rs`
- Modify: `crates/chorus-providers/src/lib.rs`

### Step 1: Create mock SMS provider with tests

```rust
// crates/chorus-providers/src/sms/mock.rs
use async_trait::async_trait;
use chorus_core::error::ChorusError;
use chorus_core::sms::SmsSender;
use chorus_core::types::{Channel, DeliveryStatus, SendResult, SmsMessage};
use chrono::Utc;
use uuid::Uuid;

/// Mock SMS provider that logs messages instead of sending.
/// Used for development and testing.
pub struct MockSmsSender;

#[async_trait]
impl SmsSender for MockSmsSender {
    fn provider_name(&self) -> &str {
        "mock"
    }

    async fn send(&self, msg: &SmsMessage) -> Result<SendResult, ChorusError> {
        tracing::info!(
            provider = "mock",
            to = %msg.to,
            body_len = msg.body.len(),
            "SMS would be sent (mock mode)"
        );

        Ok(SendResult {
            message_id: Uuid::new_v4().to_string(),
            provider: "mock".to_string(),
            channel: Channel::Sms,
            status: DeliveryStatus::Sent,
            created_at: Utc::now(),
        })
    }

    async fn check_status(&self, _message_id: &str) -> Result<DeliveryStatus, ChorusError> {
        Ok(DeliveryStatus::Delivered)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn mock_sms_send_succeeds() {
        let sender = MockSmsSender;
        let msg = SmsMessage {
            to: "+66812345678".to_string(),
            body: "Test OTP: 123456".to_string(),
            from: None,
        };
        let result = sender.send(&msg).await.unwrap();
        assert_eq!(result.provider, "mock");
        assert_eq!(result.channel, Channel::Sms);
        assert!(matches!(result.status, DeliveryStatus::Sent));
    }

    #[tokio::test]
    async fn mock_sms_check_status_returns_delivered() {
        let sender = MockSmsSender;
        let status = sender.check_status("any-id").await.unwrap();
        assert_eq!(status, DeliveryStatus::Delivered);
    }
}
```

### Step 2: Create mock Email provider with tests

```rust
// crates/chorus-providers/src/email/mock.rs
use async_trait::async_trait;
use chorus_core::email::EmailSender;
use chorus_core::error::ChorusError;
use chorus_core::types::{Channel, DeliveryStatus, EmailMessage, SendResult};
use chrono::Utc;
use uuid::Uuid;

/// Mock Email provider that logs messages instead of sending.
pub struct MockEmailSender;

#[async_trait]
impl EmailSender for MockEmailSender {
    fn provider_name(&self) -> &str {
        "mock"
    }

    async fn send(&self, msg: &EmailMessage) -> Result<SendResult, ChorusError> {
        tracing::info!(
            provider = "mock",
            to = %msg.to,
            subject = %msg.subject,
            "Email would be sent (mock mode)"
        );

        Ok(SendResult {
            message_id: Uuid::new_v4().to_string(),
            provider: "mock".to_string(),
            channel: Channel::Email,
            status: DeliveryStatus::Sent,
            created_at: Utc::now(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn mock_email_send_succeeds() {
        let sender = MockEmailSender;
        let msg = EmailMessage {
            to: "user@test.com".to_string(),
            subject: "Test".to_string(),
            html_body: "<p>Hello</p>".to_string(),
            text_body: "Hello".to_string(),
            from: None,
        };
        let result = sender.send(&msg).await.unwrap();
        assert_eq!(result.provider, "mock");
        assert_eq!(result.channel, Channel::Email);
    }
}
```

### Step 3: Wire up mod.rs files

```rust
// crates/chorus-providers/src/sms/mod.rs
pub mod mock;

// crates/chorus-providers/src/email/mod.rs
pub mod mock;

// crates/chorus-providers/src/lib.rs
pub mod sms;
pub mod email;
```

### Step 4: Verify tests pass

Run: `cargo test --workspace`
Expected: All mock tests pass

### Step 5: Commit

```bash
git commit -m "feat(providers): add MockSmsSender and MockEmailSender for dev/testing"
```

---

## Task 5: Waterfall Router — chorus-core

### Files
- Create: `crates/chorus-core/src/router.rs`
- Modify: `crates/chorus-core/src/lib.rs`

### Step 1: Implement WaterfallRouter

```rust
// crates/chorus-core/src/router.rs
use std::sync::Arc;
use crate::email::EmailSender;
use crate::error::ChorusError;
use crate::sms::SmsSender;
use crate::types::{Channel, EmailMessage, SendResult, SmsMessage};

/// A step in the waterfall routing chain.
pub struct RouteStep {
    pub channel: Channel,
    sender: RouteSender,
}

enum RouteSender {
    Sms(Arc<dyn SmsSender>),
    Email(Arc<dyn EmailSender>),
}

/// Waterfall router: tries each step in order, falls back to next on failure.
/// Optimizes cost by trying cheaper channels (email) before expensive ones (SMS).
pub struct WaterfallRouter {
    steps: Vec<RouteStep>,
}

impl WaterfallRouter {
    pub fn new() -> Self {
        Self { steps: Vec::new() }
    }

    pub fn add_sms(mut self, provider: Arc<dyn SmsSender>) -> Self {
        self.steps.push(RouteStep {
            channel: Channel::Sms,
            sender: RouteSender::Sms(provider),
        });
        self
    }

    pub fn add_email(mut self, provider: Arc<dyn EmailSender>) -> Self {
        self.steps.push(RouteStep {
            channel: Channel::Email,
            sender: RouteSender::Email(provider),
        });
        self
    }

    /// Send a message through the waterfall chain.
    /// For OTP: recipient can be email or phone — tries each step in order.
    pub async fn send_otp(
        &self,
        recipient: &str,
        code: &str,
        app_name: &str,
    ) -> Result<SendResult, ChorusError> {
        let mut errors = Vec::new();

        for step in &self.steps {
            let result = match &step.sender {
                RouteSender::Email(sender) => {
                    if !recipient.contains('@') {
                        continue; // skip email for phone numbers
                    }
                    let msg = EmailMessage {
                        to: recipient.to_string(),
                        subject: format!("Your {} verification code", app_name),
                        html_body: format!(
                            "<p>Your verification code is: <strong>{}</strong>. It expires in 5 minutes.</p>",
                            code
                        ),
                        text_body: format!(
                            "Your verification code is: {}. It expires in 5 minutes.",
                            code
                        ),
                        from: None,
                    };
                    sender.send(&msg).await
                }
                RouteSender::Sms(sender) => {
                    if recipient.contains('@') {
                        continue; // skip SMS for email addresses
                    }
                    let msg = SmsMessage {
                        to: recipient.to_string(),
                        body: format!("Your {} code: {} (expires in 5 min)", app_name, code),
                        from: None,
                    };
                    sender.send(&msg).await
                }
            };

            match result {
                Ok(send_result) => {
                    tracing::info!(
                        provider = %send_result.provider,
                        channel = %send_result.channel,
                        "Message sent successfully via waterfall"
                    );
                    return Ok(send_result);
                }
                Err(e) => {
                    tracing::warn!(
                        channel = %step.channel,
                        error = %e,
                        "Waterfall step failed, trying next"
                    );
                    errors.push(e);
                }
            }
        }

        Err(ChorusError::AllProvidersFailed)
    }

    /// Send SMS directly (bypass waterfall).
    pub async fn send_sms(&self, msg: &SmsMessage) -> Result<SendResult, ChorusError> {
        for step in &self.steps {
            if let RouteSender::Sms(sender) = &step.sender {
                match sender.send(msg).await {
                    Ok(result) => return Ok(result),
                    Err(e) => {
                        tracing::warn!(provider = sender.provider_name(), error = %e, "SMS provider failed, trying next");
                        continue;
                    }
                }
            }
        }
        Err(ChorusError::AllProvidersFailed)
    }

    /// Send email directly (bypass waterfall).
    pub async fn send_email(&self, msg: &EmailMessage) -> Result<SendResult, ChorusError> {
        for step in &self.steps {
            if let RouteSender::Email(sender) = &step.sender {
                match sender.send(msg).await {
                    Ok(result) => return Ok(result),
                    Err(e) => {
                        tracing::warn!(provider = sender.provider_name(), error = %e, "Email provider failed, trying next");
                        continue;
                    }
                }
            }
        }
        Err(ChorusError::AllProvidersFailed)
    }
}

impl Default for WaterfallRouter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::DeliveryStatus;

    // Inline test helpers — no external test deps needed

    struct SuccessSms;
    #[async_trait::async_trait]
    impl SmsSender for SuccessSms {
        fn provider_name(&self) -> &str { "test-sms" }
        async fn send(&self, _msg: &SmsMessage) -> Result<SendResult, ChorusError> {
            Ok(SendResult {
                message_id: "sms-1".to_string(),
                provider: "test-sms".to_string(),
                channel: Channel::Sms,
                status: DeliveryStatus::Sent,
                created_at: chrono::Utc::now(),
            })
        }
        async fn check_status(&self, _id: &str) -> Result<DeliveryStatus, ChorusError> {
            Ok(DeliveryStatus::Delivered)
        }
    }

    struct FailSms;
    #[async_trait::async_trait]
    impl SmsSender for FailSms {
        fn provider_name(&self) -> &str { "fail-sms" }
        async fn send(&self, _msg: &SmsMessage) -> Result<SendResult, ChorusError> {
            Err(ChorusError::Provider { provider: "fail-sms".into(), message: "timeout".into() })
        }
        async fn check_status(&self, _id: &str) -> Result<DeliveryStatus, ChorusError> {
            Ok(DeliveryStatus::Failed { reason: "timeout".into() })
        }
    }

    struct SuccessEmail;
    #[async_trait::async_trait]
    impl EmailSender for SuccessEmail {
        fn provider_name(&self) -> &str { "test-email" }
        async fn send(&self, _msg: &EmailMessage) -> Result<SendResult, ChorusError> {
            Ok(SendResult {
                message_id: "email-1".to_string(),
                provider: "test-email".to_string(),
                channel: Channel::Email,
                status: DeliveryStatus::Sent,
                created_at: chrono::Utc::now(),
            })
        }
    }

    #[tokio::test]
    async fn waterfall_sends_email_for_email_recipient() {
        let router = WaterfallRouter::new()
            .add_email(Arc::new(SuccessEmail))
            .add_sms(Arc::new(SuccessSms));

        let result = router.send_otp("user@test.com", "123456", "TestApp").await.unwrap();
        assert_eq!(result.channel, Channel::Email);
        assert_eq!(result.provider, "test-email");
    }

    #[tokio::test]
    async fn waterfall_sends_sms_for_phone_recipient() {
        let router = WaterfallRouter::new()
            .add_email(Arc::new(SuccessEmail))
            .add_sms(Arc::new(SuccessSms));

        let result = router.send_otp("+66812345678", "123456", "TestApp").await.unwrap();
        assert_eq!(result.channel, Channel::Sms);
        assert_eq!(result.provider, "test-sms");
    }

    #[tokio::test]
    async fn waterfall_fallback_on_failure() {
        let router = WaterfallRouter::new()
            .add_sms(Arc::new(FailSms))
            .add_sms(Arc::new(SuccessSms));

        let result = router.send_otp("+66812345678", "123456", "TestApp").await.unwrap();
        assert_eq!(result.provider, "test-sms"); // fell through to second provider
    }

    #[tokio::test]
    async fn waterfall_all_fail_returns_error() {
        let router = WaterfallRouter::new()
            .add_sms(Arc::new(FailSms));

        let result = router.send_otp("+66812345678", "123456", "TestApp").await;
        assert!(matches!(result, Err(ChorusError::AllProvidersFailed)));
    }

    #[tokio::test]
    async fn waterfall_empty_router_returns_error() {
        let router = WaterfallRouter::new();
        let result = router.send_otp("user@test.com", "123456", "TestApp").await;
        assert!(matches!(result, Err(ChorusError::AllProvidersFailed)));
    }

    #[tokio::test]
    async fn send_sms_directly() {
        let router = WaterfallRouter::new()
            .add_email(Arc::new(SuccessEmail))
            .add_sms(Arc::new(SuccessSms));

        let msg = SmsMessage { to: "+66812345678".into(), body: "Hi".into(), from: None };
        let result = router.send_sms(&msg).await.unwrap();
        assert_eq!(result.channel, Channel::Sms);
    }

    #[tokio::test]
    async fn send_email_directly() {
        let router = WaterfallRouter::new()
            .add_email(Arc::new(SuccessEmail))
            .add_sms(Arc::new(SuccessSms));

        let msg = EmailMessage {
            to: "user@test.com".into(), subject: "Hi".into(),
            html_body: "<p>Hi</p>".into(), text_body: "Hi".into(), from: None,
        };
        let result = router.send_email(&msg).await.unwrap();
        assert_eq!(result.channel, Channel::Email);
    }
}
```

### Step 2: Update lib.rs

```rust
// crates/chorus-core/src/lib.rs
pub mod email;
pub mod error;
pub mod router;
pub mod sms;
pub mod types;
```

### Step 3: Verify tests pass

Run: `cargo test --workspace`
Expected: All router tests + previous tests pass

### Step 4: Commit

```bash
git commit -m "feat(core): add WaterfallRouter with email-first/SMS-fallback and multi-provider failover"
```

---

## Task 6: Telnyx SMS Provider

### Files
- Create: `crates/chorus-providers/src/sms/telnyx.rs`
- Modify: `crates/chorus-providers/src/sms/mod.rs`

### Step 1: Implement TelnyxSmsSender

```rust
// crates/chorus-providers/src/sms/telnyx.rs
use async_trait::async_trait;
use chorus_core::error::ChorusError;
use chorus_core::sms::SmsSender;
use chorus_core::types::{Channel, DeliveryStatus, SendResult, SmsMessage};
use chrono::Utc;
use serde::Deserialize;

pub struct TelnyxSmsSender {
    api_key: String,
    from: Option<String>,
    http_client: reqwest::Client,
}

impl TelnyxSmsSender {
    pub fn new(api_key: String, from: Option<String>) -> Self {
        Self {
            api_key,
            from,
            http_client: reqwest::Client::new(),
        }
    }
}

#[derive(Deserialize)]
struct TelnyxResponse {
    data: TelnyxMessageData,
}

#[derive(Deserialize)]
struct TelnyxMessageData {
    id: String,
}

#[async_trait]
impl SmsSender for TelnyxSmsSender {
    fn provider_name(&self) -> &str {
        "telnyx"
    }

    async fn send(&self, msg: &SmsMessage) -> Result<SendResult, ChorusError> {
        let from = msg.from.as_ref().or(self.from.as_ref()).ok_or_else(|| {
            ChorusError::Validation("SMS 'from' number is required for Telnyx".to_string())
        })?;

        let payload = serde_json::json!({
            "from": from,
            "to": msg.to,
            "text": msg.body,
        });

        let resp = self
            .http_client
            .post("https://api.telnyx.com/v2/messages")
            .bearer_auth(&self.api_key)
            .json(&payload)
            .send()
            .await
            .map_err(|e| ChorusError::Provider {
                provider: "telnyx".into(),
                message: format!("HTTP error: {}", e),
            })?;

        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(ChorusError::Provider {
                provider: "telnyx".into(),
                message: format!("API error: {}", body),
            });
        }

        let telnyx_resp: TelnyxResponse = resp.json().await.map_err(|e| {
            ChorusError::Provider {
                provider: "telnyx".into(),
                message: format!("parse error: {}", e),
            }
        })?;

        Ok(SendResult {
            message_id: telnyx_resp.data.id,
            provider: "telnyx".to_string(),
            channel: Channel::Sms,
            status: DeliveryStatus::Queued,
            created_at: Utc::now(),
        })
    }

    async fn check_status(&self, _message_id: &str) -> Result<DeliveryStatus, ChorusError> {
        // Telnyx delivery status comes via webhooks — return Sent as default
        Ok(DeliveryStatus::Sent)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn telnyx_provider_name() {
        let sender = TelnyxSmsSender::new("fake-key".into(), Some("+1234567890".into()));
        assert_eq!(sender.provider_name(), "telnyx");
    }

    #[tokio::test]
    async fn telnyx_requires_from_number() {
        let sender = TelnyxSmsSender::new("fake-key".into(), None);
        let msg = SmsMessage { to: "+66812345678".into(), body: "Hi".into(), from: None };
        let result = sender.send(&msg).await;
        assert!(matches!(result, Err(ChorusError::Validation(_))));
    }
}
```

### Step 2: Update mod.rs

```rust
// crates/chorus-providers/src/sms/mod.rs
pub mod mock;
pub mod telnyx;
```

### Step 3: Verify tests pass

Run: `cargo test --workspace`

### Step 4: Commit

```bash
git commit -m "feat(providers): add TelnyxSmsSender"
```

---

## Task 7: Twilio SMS Provider

### Files
- Create: `crates/chorus-providers/src/sms/twilio.rs`
- Modify: `crates/chorus-providers/src/sms/mod.rs`

### Step 1: Implement TwilioSmsSender

Same pattern as Telnyx but with:
- Basic auth (`account_sid:auth_token`)
- POST to `https://api.twilio.com/2010-04-01/Accounts/{sid}/Messages.json`
- Form-encoded body (`To`, `From`, `Body`)
- Response: `{ "sid": "SMxxx" }`

### Step 2: Tests (provider name, missing from number)

### Step 3: Commit

```bash
git commit -m "feat(providers): add TwilioSmsSender"
```

---

## Task 8: Plivo SMS Provider

### Files
- Create: `crates/chorus-providers/src/sms/plivo.rs`
- Modify: `crates/chorus-providers/src/sms/mod.rs`

### Step 1: Implement PlivoSmsSender

Same pattern but with:
- Basic auth (`auth_id:auth_token`)
- POST to `https://api.plivo.com/v1/Account/{auth_id}/Message/`
- JSON body (`src`, `dst`, `text`)
- Response: `{ "message_uuid": ["xxx"] }`

### Step 2: Tests and commit

```bash
git commit -m "feat(providers): add PlivoSmsSender"
```

---

## Task 9: Resend Email Provider

### Files
- Create: `crates/chorus-providers/src/email/resend.rs`
- Modify: `crates/chorus-providers/src/email/mod.rs`

### Step 1: Implement ResendEmailSender

```rust
// crates/chorus-providers/src/email/resend.rs
use async_trait::async_trait;
use chorus_core::email::EmailSender;
use chorus_core::error::ChorusError;
use chorus_core::types::{Channel, DeliveryStatus, EmailMessage, SendResult};
use chrono::Utc;
use serde::Deserialize;

pub struct ResendEmailSender {
    api_key: String,
    from: String,
    http_client: reqwest::Client,
}

impl ResendEmailSender {
    pub fn new(api_key: String, from: String) -> Self {
        Self {
            api_key,
            from,
            http_client: reqwest::Client::new(),
        }
    }
}

#[derive(Deserialize)]
struct ResendResponse {
    id: String,
}

#[async_trait]
impl EmailSender for ResendEmailSender {
    fn provider_name(&self) -> &str {
        "resend"
    }

    async fn send(&self, msg: &EmailMessage) -> Result<SendResult, ChorusError> {
        let from = msg.from.as_deref().unwrap_or(&self.from);

        let payload = serde_json::json!({
            "from": from,
            "to": [msg.to],
            "subject": msg.subject,
            "html": msg.html_body,
            "text": msg.text_body,
        });

        let resp = self
            .http_client
            .post("https://api.resend.com/emails")
            .bearer_auth(&self.api_key)
            .json(&payload)
            .send()
            .await
            .map_err(|e| ChorusError::Provider {
                provider: "resend".into(),
                message: format!("HTTP error: {}", e),
            })?;

        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(ChorusError::Provider {
                provider: "resend".into(),
                message: format!("API error: {}", body),
            });
        }

        let resend_resp: ResendResponse = resp.json().await.map_err(|e| {
            ChorusError::Provider {
                provider: "resend".into(),
                message: format!("parse error: {}", e),
            }
        })?;

        Ok(SendResult {
            message_id: resend_resp.id,
            provider: "resend".to_string(),
            channel: Channel::Email,
            status: DeliveryStatus::Sent,
            created_at: Utc::now(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resend_provider_name() {
        let sender = ResendEmailSender::new("re_xxx".into(), "noreply@test.com".into());
        assert_eq!(sender.provider_name(), "resend");
    }
}
```

### Step 2: Update mod.rs and commit

```bash
git commit -m "feat(providers): add ResendEmailSender"
```

---

## Task 10: AWS SES Email Provider

### Files
- Create: `crates/chorus-providers/src/email/ses.rs`
- Modify: `crates/chorus-providers/src/email/mod.rs`

### Step 1: Implement SesEmailSender

Uses SES v2 API:
- POST to `https://email.{region}.amazonaws.com/v2/email/outbound-emails`
- AWS Signature V4 auth (or use `aws-sdk-sesv2` crate)
- JSON body with `Destination`, `Content`, `FromEmailAddress`

For simplicity, use SES SMTP relay as initial implementation (simpler than SigV4):
- Connect via SMTP to `email-smtp.{region}.amazonaws.com:587`
- TLS + PLAIN auth with SES SMTP credentials

### Step 2: Tests and commit

```bash
git commit -m "feat(providers): add SesEmailSender (SMTP mode)"
```

---

## Task 11: SMTP Email Provider

### Files
- Create: `crates/chorus-providers/src/email/smtp.rs`
- Modify: `crates/chorus-providers/src/email/mod.rs`

### Step 1: Implement SmtpEmailSender

Add `lettre` to workspace dependencies (Rust SMTP library):

```toml
# Cargo.toml workspace
lettre = { version = "0.11", features = ["tokio1-native-tls"] }
```

Uses `lettre` crate for SMTP:
- Connect to any SMTP server (Gmail, SES, Postfix, etc.)
- TLS/STARTTLS support
- Configurable: host, port, username, password

### Step 2: Tests and commit

```bash
git commit -m "feat(providers): add SmtpEmailSender (universal SMTP)"
```

---

## Task 12: Template Engine — chorus-core

### Files
- Create: `crates/chorus-core/src/template.rs`
- Modify: `crates/chorus-core/src/lib.rs`

### Step 1: Implement simple template engine

```rust
// crates/chorus-core/src/template.rs
use crate::error::ChorusError;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Template {
    pub slug: String,
    pub name: String,
    pub subject: String,
    pub html_body: String,
    pub text_body: String,
    pub variables: Vec<String>,
}

impl Template {
    /// Render template by replacing {{variable}} placeholders with values.
    pub fn render(
        &self,
        variables: &HashMap<String, String>,
    ) -> Result<RenderedTemplate, ChorusError> {
        let subject = Self::replace_vars(&self.subject, variables);
        let html_body = Self::replace_vars(&self.html_body, variables);
        let text_body = Self::replace_vars(&self.text_body, variables);

        Ok(RenderedTemplate {
            subject,
            html_body,
            text_body,
        })
    }

    fn replace_vars(text: &str, variables: &HashMap<String, String>) -> String {
        let mut result = text.to_string();
        for (key, value) in variables {
            result = result.replace(&format!("{{{{{}}}}}", key), value);
        }
        result
    }
}

#[derive(Debug, Clone)]
pub struct RenderedTemplate {
    pub subject: String,
    pub html_body: String,
    pub text_body: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_template() -> Template {
        Template {
            slug: "otp".to_string(),
            name: "OTP Email".to_string(),
            subject: "Your {{app_name}} code".to_string(),
            html_body: "<p>Code: <strong>{{code}}</strong>. Expires in {{expire}} min.</p>"
                .to_string(),
            text_body: "Code: {{code}}. Expires in {{expire}} min.".to_string(),
            variables: vec![
                "code".to_string(),
                "app_name".to_string(),
                "expire".to_string(),
            ],
        }
    }

    #[test]
    fn render_replaces_all_variables() {
        let tmpl = test_template();
        let mut vars = HashMap::new();
        vars.insert("code".to_string(), "123456".to_string());
        vars.insert("app_name".to_string(), "Orbit".to_string());
        vars.insert("expire".to_string(), "5".to_string());

        let rendered = tmpl.render(&vars).unwrap();
        assert_eq!(rendered.subject, "Your Orbit code");
        assert!(rendered.html_body.contains("<strong>123456</strong>"));
        assert!(rendered.text_body.contains("123456"));
        assert!(rendered.text_body.contains("5 min"));
    }

    #[test]
    fn render_leaves_unknown_vars_as_is() {
        let tmpl = test_template();
        let vars = HashMap::new(); // no variables provided
        let rendered = tmpl.render(&vars).unwrap();
        assert!(rendered.subject.contains("{{app_name}}"));
    }

    #[test]
    fn render_handles_repeated_variable() {
        let tmpl = Template {
            slug: "test".into(),
            name: "Test".into(),
            subject: "{{code}} is your code {{code}}".into(),
            html_body: "".into(),
            text_body: "".into(),
            variables: vec!["code".into()],
        };
        let mut vars = HashMap::new();
        vars.insert("code".into(), "999".into());
        let rendered = tmpl.render(&vars).unwrap();
        assert_eq!(rendered.subject, "999 is your code 999");
    }
}
```

### Step 2: Update lib.rs and commit

```bash
git commit -m "feat(core): add Template engine with {{variable}} rendering"
```

---

## Task 13: Chorus Client — High-level API

### Files
- Create: `crates/chorus-core/src/client.rs`
- Modify: `crates/chorus-core/src/lib.rs`

### Step 1: Implement Chorus client (the main entry point)

```rust
// crates/chorus-core/src/client.rs
use std::sync::Arc;
use crate::email::EmailSender;
use crate::error::ChorusError;
use crate::router::WaterfallRouter;
use crate::sms::SmsSender;
use crate::template::Template;
use crate::types::{EmailMessage, SendResult, SmsMessage};
use std::collections::HashMap;

/// The main Chorus client — high-level API for sending messages.
pub struct Chorus {
    router: WaterfallRouter,
    templates: HashMap<String, Template>,
    default_from_email: Option<String>,
    default_from_sms: Option<String>,
}

impl Chorus {
    pub fn builder() -> ChorusBuilder {
        ChorusBuilder::new()
    }

    pub async fn send_sms(&self, msg: &SmsMessage) -> Result<SendResult, ChorusError> {
        self.router.send_sms(msg).await
    }

    pub async fn send_email(&self, msg: &EmailMessage) -> Result<SendResult, ChorusError> {
        self.router.send_email(msg).await
    }

    pub async fn send_email_template(
        &self,
        to: &str,
        template_slug: &str,
        variables: &HashMap<String, String>,
    ) -> Result<SendResult, ChorusError> {
        let tmpl = self
            .templates
            .get(template_slug)
            .ok_or_else(|| ChorusError::TemplateNotFound(template_slug.to_string()))?;

        let rendered = tmpl.render(variables)?;

        let msg = EmailMessage {
            to: to.to_string(),
            subject: rendered.subject,
            html_body: rendered.html_body,
            text_body: rendered.text_body,
            from: self.default_from_email.clone(),
        };

        self.router.send_email(&msg).await
    }

    pub async fn send_otp(
        &self,
        recipient: &str,
        code: &str,
        app_name: &str,
    ) -> Result<SendResult, ChorusError> {
        self.router.send_otp(recipient, code, app_name).await
    }
}

pub struct ChorusBuilder {
    router: WaterfallRouter,
    templates: HashMap<String, Template>,
    default_from_email: Option<String>,
    default_from_sms: Option<String>,
}

impl ChorusBuilder {
    pub fn new() -> Self {
        Self {
            router: WaterfallRouter::new(),
            templates: HashMap::new(),
            default_from_email: None,
            default_from_sms: None,
        }
    }

    pub fn add_sms_provider(mut self, provider: Arc<dyn SmsSender>) -> Self {
        self.router = self.router.add_sms(provider);
        self
    }

    pub fn add_email_provider(mut self, provider: Arc<dyn EmailSender>) -> Self {
        self.router = self.router.add_email(provider);
        self
    }

    pub fn add_template(mut self, template: Template) -> Self {
        self.templates.insert(template.slug.clone(), template);
        self
    }

    pub fn default_from_email(mut self, from: String) -> Self {
        self.default_from_email = Some(from);
        self
    }

    pub fn default_from_sms(mut self, from: String) -> Self {
        self.default_from_sms = Some(from);
        self
    }

    pub fn build(self) -> Chorus {
        Chorus {
            router: self.router,
            templates: self.templates,
            default_from_email: self.default_from_email,
            default_from_sms: self.default_from_sms,
        }
    }
}

impl Default for ChorusBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Channel, DeliveryStatus};

    // Reuse test helpers
    struct TestEmail;
    #[async_trait::async_trait]
    impl EmailSender for TestEmail {
        fn provider_name(&self) -> &str { "test" }
        async fn send(&self, _msg: &EmailMessage) -> Result<SendResult, ChorusError> {
            Ok(SendResult {
                message_id: "e1".into(), provider: "test".into(),
                channel: Channel::Email, status: DeliveryStatus::Sent,
                created_at: chrono::Utc::now(),
            })
        }
    }

    struct TestSms;
    #[async_trait::async_trait]
    impl SmsSender for TestSms {
        fn provider_name(&self) -> &str { "test" }
        async fn send(&self, _msg: &SmsMessage) -> Result<SendResult, ChorusError> {
            Ok(SendResult {
                message_id: "s1".into(), provider: "test".into(),
                channel: Channel::Sms, status: DeliveryStatus::Sent,
                created_at: chrono::Utc::now(),
            })
        }
        async fn check_status(&self, _id: &str) -> Result<DeliveryStatus, ChorusError> {
            Ok(DeliveryStatus::Delivered)
        }
    }

    #[tokio::test]
    async fn chorus_send_email_template() {
        let chorus = Chorus::builder()
            .add_email_provider(Arc::new(TestEmail))
            .add_template(Template {
                slug: "otp".into(), name: "OTP".into(),
                subject: "Code: {{code}}".into(),
                html_body: "<p>{{code}}</p>".into(),
                text_body: "{{code}}".into(),
                variables: vec!["code".into()],
            })
            .build();

        let mut vars = HashMap::new();
        vars.insert("code".into(), "123456".into());

        let result = chorus.send_email_template("user@test.com", "otp", &vars).await.unwrap();
        assert_eq!(result.channel, Channel::Email);
    }

    #[tokio::test]
    async fn chorus_template_not_found() {
        let chorus = Chorus::builder()
            .add_email_provider(Arc::new(TestEmail))
            .build();

        let vars = HashMap::new();
        let result = chorus.send_email_template("user@test.com", "nonexistent", &vars).await;
        assert!(matches!(result, Err(ChorusError::TemplateNotFound(_))));
    }

    #[tokio::test]
    async fn chorus_send_otp_email() {
        let chorus = Chorus::builder()
            .add_email_provider(Arc::new(TestEmail))
            .add_sms_provider(Arc::new(TestSms))
            .build();

        let result = chorus.send_otp("user@test.com", "123456", "App").await.unwrap();
        assert_eq!(result.channel, Channel::Email);
    }

    #[tokio::test]
    async fn chorus_send_otp_sms() {
        let chorus = Chorus::builder()
            .add_email_provider(Arc::new(TestEmail))
            .add_sms_provider(Arc::new(TestSms))
            .build();

        let result = chorus.send_otp("+66812345678", "123456", "App").await.unwrap();
        assert_eq!(result.channel, Channel::Sms);
    }
}
```

### Step 2: Update lib.rs and commit

```bash
git commit -m "feat(core): add Chorus client with builder pattern, template rendering, and OTP"
```

---

## Verification Checklist

After all tasks complete:

```bash
# Full test suite
cargo test --workspace

# Lint
cargo clippy --workspace -- -D warnings

# Format
cargo fmt --all

# Verify crate structure
ls crates/chorus-core/src/
# Expected: lib.rs client.rs email.rs error.rs router.rs sms.rs template.rs types.rs

ls crates/chorus-providers/src/sms/
# Expected: mod.rs mock.rs telnyx.rs twilio.rs plivo.rs

ls crates/chorus-providers/src/email/
# Expected: mod.rs mock.rs resend.rs ses.rs smtp.rs
```

All must pass before PR.
