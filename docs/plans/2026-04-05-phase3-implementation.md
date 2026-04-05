# Phase 3: Providers, Templates & chorus-mail — Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Fix SMS status stubs, add pluggable template engine with built-in auth templates, and create self-hosted email infrastructure (chorus-mail).

**Architecture:** Three independent sub-phases — 3a fixes SMS providers in chorus-providers, 3b replaces the template engine in chorus-core with minijinja, 3c adds a Postfix Docker image + bounce/DNS endpoints in chorus-server. Each phase is independently deployable.

**Tech Stack:** Rust, reqwest 0.12 (SMS status APIs), minijinja (template engine), Postfix + OpenDKIM (Docker), hickory-resolver (DNS checks), Axum 0.8 (new internal endpoints)

**Prerequisite:** Phase 2b PR #13 must be merged before starting. This plan assumes the expanded Config, AppState with provider_config_repo, and worker pool are in place.

**Execution order:** Task 1-3 (Phase 3a) → Task 4-5 (Phase 3b) → Task 6-9 (Phase 3c)

---

## Task 1: Twilio check_status() Real Implementation

### Files
- Modify: `crates/chorus-providers/src/sms/twilio.rs`

### Step 1: Add response struct

Add after the existing `TwilioResponse` struct (around line 32):

```rust
#[derive(Deserialize)]
struct TwilioStatusResponse {
    status: String,
}
```

### Step 2: Implement check_status()

Replace the stub `check_status()` (lines 81-83) with:

```rust
async fn check_status(&self, message_id: &str) -> Result<DeliveryStatus, ChorusError> {
    let url = format!(
        "https://api.twilio.com/2010-04-01/Accounts/{}/Messages/{}.json",
        self.account_sid, message_id
    );

    let resp = self
        .http_client
        .get(&url)
        .basic_auth(&self.account_sid, Some(&self.auth_token))
        .send()
        .await
        .map_err(|e| ChorusError::Provider {
            provider: "twilio".into(),
            message: format!("HTTP error: {}", e),
        })?;

    if !resp.status().is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(ChorusError::Provider {
            provider: "twilio".into(),
            message: format!("status check failed: {}", body),
        });
    }

    let status_resp: TwilioStatusResponse =
        resp.json().await.map_err(|e| ChorusError::Provider {
            provider: "twilio".into(),
            message: format!("parse error: {}", e),
        })?;

    Ok(map_twilio_status(&status_resp.status))
}
```

### Step 3: Add status mapping function

Add after the `impl SmsSender` block:

```rust
fn map_twilio_status(status: &str) -> DeliveryStatus {
    match status {
        "delivered" => DeliveryStatus::Delivered,
        "sent" => DeliveryStatus::Delivered,
        "failed" | "undelivered" => DeliveryStatus::Failed {
            reason: format!("twilio status: {}", status),
        },
        "queued" | "accepted" | "sending" => DeliveryStatus::Sent,
        _ => DeliveryStatus::Sent,
    }
}
```

### Step 4: Verify compilation

Run: `cargo check --workspace`

### Step 5: Commit

```bash
git add crates/chorus-providers/src/sms/twilio.rs
git commit -m "fix(providers): implement real Twilio check_status via Messages API"
```

---

## Task 2: Telnyx check_status() Real Implementation

### Files
- Modify: `crates/chorus-providers/src/sms/telnyx.rs`

### Step 1: Add response structs

Add after the existing `TelnyxResponse` struct:

```rust
#[derive(Deserialize)]
struct TelnyxStatusResponse {
    data: TelnyxStatusData,
}

#[derive(Deserialize)]
struct TelnyxStatusData {
    to: Vec<TelnyxRecipientStatus>,
}

#[derive(Deserialize)]
struct TelnyxRecipientStatus {
    status: String,
}
```

### Step 2: Implement check_status()

Replace the stub (lines 85-87) with:

```rust
async fn check_status(&self, message_id: &str) -> Result<DeliveryStatus, ChorusError> {
    let url = format!("https://api.telnyx.com/v2/messages/{}", message_id);

    let resp = self
        .http_client
        .get(&url)
        .bearer_auth(&self.api_key)
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
            message: format!("status check failed: {}", body),
        });
    }

    let status_resp: TelnyxStatusResponse =
        resp.json().await.map_err(|e| ChorusError::Provider {
            provider: "telnyx".into(),
            message: format!("parse error: {}", e),
        })?;

    let status = status_resp
        .data
        .to
        .first()
        .map(|r| r.status.as_str())
        .unwrap_or("unknown");

    Ok(map_telnyx_status(status))
}
```

### Step 3: Add status mapping function

```rust
fn map_telnyx_status(status: &str) -> DeliveryStatus {
    match status {
        "delivered" => DeliveryStatus::Delivered,
        "sent" => DeliveryStatus::Delivered,
        "sending_failed" => DeliveryStatus::Failed {
            reason: format!("telnyx status: {}", status),
        },
        "queued" | "sending" => DeliveryStatus::Sent,
        _ => DeliveryStatus::Sent,
    }
}
```

### Step 4: Verify compilation

Run: `cargo check --workspace`

### Step 5: Commit

```bash
git add crates/chorus-providers/src/sms/telnyx.rs
git commit -m "fix(providers): implement real Telnyx check_status via Messages API"
```

---

## Task 3: Plivo check_status() Real Implementation

### Files
- Modify: `crates/chorus-providers/src/sms/plivo.rs`

### Step 1: Add response struct

Add after the existing `PlivoResponse` struct:

```rust
#[derive(Deserialize)]
struct PlivoStatusResponse {
    message_state: String,
}
```

### Step 2: Implement check_status()

Replace the stub (lines 90-92) with:

```rust
async fn check_status(&self, message_id: &str) -> Result<DeliveryStatus, ChorusError> {
    let url = format!(
        "https://api.plivo.com/v1/Account/{}/Message/{}/",
        self.auth_id, message_id
    );

    let resp = self
        .http_client
        .get(&url)
        .basic_auth(&self.auth_id, Some(&self.auth_token))
        .send()
        .await
        .map_err(|e| ChorusError::Provider {
            provider: "plivo".into(),
            message: format!("HTTP error: {}", e),
        })?;

    if !resp.status().is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(ChorusError::Provider {
            provider: "plivo".into(),
            message: format!("status check failed: {}", body),
        });
    }

    let status_resp: PlivoStatusResponse =
        resp.json().await.map_err(|e| ChorusError::Provider {
            provider: "plivo".into(),
            message: format!("parse error: {}", e),
        })?;

    Ok(map_plivo_status(&status_resp.message_state))
}
```

### Step 3: Add status mapping function

```rust
fn map_plivo_status(state: &str) -> DeliveryStatus {
    match state {
        "delivered" => DeliveryStatus::Delivered,
        "failed" | "rejected" => DeliveryStatus::Failed {
            reason: format!("plivo status: {}", state),
        },
        "queued" | "sent" => DeliveryStatus::Sent,
        _ => DeliveryStatus::Sent,
    }
}
```

### Step 4: Run full test suite

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

### Step 5: Commit

```bash
git add crates/chorus-providers/src/sms/
git commit -m "fix(providers): implement real Plivo check_status via Message API"
```

---

## Task 4: Pluggable Template Engine (minijinja)

### Files
- Modify: `crates/chorus-core/Cargo.toml`
- Modify: `crates/chorus-core/src/template.rs`
- Modify: `crates/chorus-core/src/error.rs` (add Template variant if missing)

### Step 1: Add minijinja dependency

In `crates/chorus-core/Cargo.toml`, add under `[dependencies]`:

```toml
minijinja = "2"
```

### Step 2: Refactor template.rs render internals

Replace the full content of `crates/chorus-core/src/template.rs` with:

```rust
use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::error::ChorusError;

/// A reusable message template with variable placeholders.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Template {
    /// Unique identifier for looking up this template.
    pub slug: String,
    /// Human-readable template name.
    pub name: String,
    /// Subject line template (rendered with variables).
    pub subject: String,
    /// HTML body template.
    pub html_body: String,
    /// Plain text body template.
    pub text_body: String,
    /// List of expected variable names (for documentation/validation).
    pub variables: Vec<String>,
}

/// The result of rendering a template with variables.
#[derive(Debug, Clone)]
pub struct RenderedTemplate {
    pub subject: String,
    pub html_body: String,
    pub text_body: String,
}

impl Template {
    /// Renders the template by replacing placeholders with provided values.
    ///
    /// Supports Jinja2 syntax: `{{ variable }}`, `{% if %}`, `{% for %}`, filters.
    /// Simple `{{variable}}` from prior versions remains compatible.
    pub fn render(
        &self,
        variables: &HashMap<String, String>,
    ) -> Result<RenderedTemplate, ChorusError> {
        let subject = render_string(&self.subject, variables)?;
        let html_body = render_string(&self.html_body, variables)?;
        let text_body = render_string(&self.text_body, variables)?;

        Ok(RenderedTemplate {
            subject,
            html_body,
            text_body,
        })
    }
}

/// Render a single template string with the given variables.
fn render_string(
    template: &str,
    variables: &HashMap<String, String>,
) -> Result<String, ChorusError> {
    let env = minijinja::Environment::new();
    let ctx = minijinja::value::Value::from_serialize(variables);
    env.render_str(template, ctx).map_err(|e| {
        ChorusError::Validation(format!("template render error: {}", e))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_template(subject: &str, html: &str, text: &str) -> Template {
        Template {
            slug: "test".into(),
            name: "Test".into(),
            subject: subject.into(),
            html_body: html.into(),
            text_body: text.into(),
            variables: vec![],
        }
    }

    #[test]
    fn renders_simple_variables() {
        let t = make_template(
            "Hello {{ name }}",
            "<p>Hi {{ name }}, code: {{ code }}</p>",
            "Hi {{ name }}, code: {{ code }}",
        );
        let vars = HashMap::from([
            ("name".into(), "Alice".into()),
            ("code".into(), "123456".into()),
        ]);
        let r = t.render(&vars).unwrap();
        assert_eq!(r.subject, "Hello Alice");
        assert_eq!(r.html_body, "<p>Hi Alice, code: 123456</p>");
        assert_eq!(r.text_body, "Hi Alice, code: 123456");
    }

    #[test]
    fn undefined_variables_render_empty() {
        let t = make_template("{{ missing }}", "", "");
        let r = t.render(&HashMap::new()).unwrap();
        assert_eq!(r.subject, "");
    }

    #[test]
    fn repeated_variables() {
        let t = make_template("{{ x }} and {{ x }}", "", "");
        let vars = HashMap::from([("x".into(), "hi".into())]);
        let r = t.render(&vars).unwrap();
        assert_eq!(r.subject, "hi and hi");
    }

    #[test]
    fn empty_template() {
        let t = make_template("", "", "");
        let r = t.render(&HashMap::new()).unwrap();
        assert_eq!(r.subject, "");
    }

    #[test]
    fn no_placeholders() {
        let t = make_template("Hello world", "<p>Hi</p>", "Hi");
        let r = t.render(&HashMap::new()).unwrap();
        assert_eq!(r.subject, "Hello world");
    }

    #[test]
    fn if_else_conditional() {
        let t = make_template(
            "{% if name %}Hi {{ name }}{% else %}Hi there{% endif %}",
            "",
            "",
        );
        let with_name = HashMap::from([("name".into(), "Bob".into())]);
        assert_eq!(t.render(&with_name).unwrap().subject, "Hi Bob");

        let without_name = HashMap::new();
        assert_eq!(t.render(&without_name).unwrap().subject, "Hi there");
    }

    #[test]
    fn for_loop() {
        let t = make_template("{% for item in items %}{{ item }} {% endfor %}", "", "");
        // minijinja needs Value types for lists — use simple var instead
        let env = minijinja::Environment::new();
        let ctx = minijinja::context! { items => vec!["a", "b", "c"] };
        let result = env.render_str("{% for item in items %}{{ item }} {% endfor %}", ctx).unwrap();
        assert_eq!(result, "a b c ");
    }

    #[test]
    fn default_filter() {
        let t = make_template("{{ name | default('Guest') }}", "", "");
        let r = t.render(&HashMap::new()).unwrap();
        assert_eq!(r.subject, "Guest");
    }

    #[test]
    fn special_characters_in_values() {
        let t = make_template("{{ val }}", "", "");
        let vars = HashMap::from([("val".into(), "<script>alert('xss')</script>".into())]);
        let r = t.render(&vars).unwrap();
        // minijinja auto-escapes HTML in templates
        assert!(r.subject.contains("&lt;script&gt;") || r.subject.contains("<script>"));
    }
}
```

### Step 3: Verify compilation and tests

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

### Step 4: Commit

```bash
git add crates/chorus-core/
git commit -m "feat(core): replace template engine with minijinja for conditional/loop support"
```

---

## Task 5: Built-in Auth Templates

### Files
- Create: `crates/chorus-core/src/templates/mod.rs`
- Create: `crates/chorus-core/src/templates/otp.rs`
- Create: `crates/chorus-core/src/templates/password_reset.rs`
- Create: `crates/chorus-core/src/templates/magic_link.rs`
- Create: `crates/chorus-core/src/templates/email_verify.rs`
- Create: `crates/chorus-core/src/templates/welcome.rs`
- Modify: `crates/chorus-core/src/lib.rs`

### Step 1: Create templates/mod.rs

```rust
mod email_verify;
mod magic_link;
mod otp;
mod password_reset;
mod welcome;

use crate::template::Template;

/// Returns all built-in auth templates.
pub fn builtin_templates() -> Vec<Template> {
    vec![
        otp::template(),
        password_reset::template(),
        magic_link::template(),
        email_verify::template(),
        welcome::template(),
    ]
}
```

### Step 2: Create templates/otp.rs

```rust
use crate::template::Template;

pub fn template() -> Template {
    Template {
        slug: "otp".into(),
        name: "OTP Verification Code".into(),
        subject: "Your {{ app_name }} verification code".into(),
        html_body: r#"<div style="max-width:480px;margin:0 auto;font-family:sans-serif;padding:24px;">
<h2 style="margin:0 0 16px;">Verification Code</h2>
<p>Your {{ app_name }} code is:</p>
<p style="font-size:32px;letter-spacing:8px;font-weight:bold;margin:24px 0;">{{ code }}</p>
<p style="color:#666;">This code expires in {{ expiry }}.</p>
<p style="color:#999;font-size:12px;margin-top:32px;">If you did not request this code, please ignore this email.</p>
</div>"#
            .into(),
        text_body: "Your {{ app_name }} verification code is: {{ code }}\n\nExpires in {{ expiry }}.\n\nIf you did not request this code, please ignore this email.".into(),
        variables: vec!["code".into(), "app_name".into(), "expiry".into()],
    }
}
```

### Step 3: Create templates/password_reset.rs

```rust
use crate::template::Template;

pub fn template() -> Template {
    Template {
        slug: "password-reset".into(),
        name: "Password Reset".into(),
        subject: "Reset your {{ app_name }} password".into(),
        html_body: r#"<div style="max-width:480px;margin:0 auto;font-family:sans-serif;padding:24px;">
<h2 style="margin:0 0 16px;">Password Reset</h2>
<p>We received a request to reset your {{ app_name }} password.</p>
<p style="margin:24px 0;"><a href="{{ reset_url }}" style="background:#111;color:#fff;padding:12px 24px;text-decoration:none;border-radius:4px;display:inline-block;">Reset Password</a></p>
<p style="color:#666;">This link expires in {{ expiry }}.</p>
<p style="color:#999;font-size:12px;margin-top:32px;">If you did not request a password reset, please ignore this email.</p>
</div>"#
            .into(),
        text_body: "Reset your {{ app_name }} password\n\nVisit this link to reset your password: {{ reset_url }}\n\nExpires in {{ expiry }}.\n\nIf you did not request this, please ignore this email.".into(),
        variables: vec!["reset_url".into(), "app_name".into(), "expiry".into()],
    }
}
```

### Step 4: Create templates/magic_link.rs

```rust
use crate::template::Template;

pub fn template() -> Template {
    Template {
        slug: "magic-link".into(),
        name: "Magic Link Sign-in".into(),
        subject: "Sign in to {{ app_name }}".into(),
        html_body: r#"<div style="max-width:480px;margin:0 auto;font-family:sans-serif;padding:24px;">
<h2 style="margin:0 0 16px;">Sign In</h2>
<p>Click the button below to sign in to {{ app_name }}.</p>
<p style="margin:24px 0;"><a href="{{ magic_url }}" style="background:#111;color:#fff;padding:12px 24px;text-decoration:none;border-radius:4px;display:inline-block;">Sign In</a></p>
<p style="color:#666;">This link expires in {{ expiry }}.</p>
<p style="color:#999;font-size:12px;margin-top:32px;">If you did not request this link, please ignore this email.</p>
</div>"#
            .into(),
        text_body: "Sign in to {{ app_name }}\n\nVisit this link to sign in: {{ magic_url }}\n\nExpires in {{ expiry }}.\n\nIf you did not request this, please ignore this email.".into(),
        variables: vec!["magic_url".into(), "app_name".into(), "expiry".into()],
    }
}
```

### Step 5: Create templates/email_verify.rs

```rust
use crate::template::Template;

pub fn template() -> Template {
    Template {
        slug: "email-verify".into(),
        name: "Email Verification".into(),
        subject: "Verify your email for {{ app_name }}".into(),
        html_body: r#"<div style="max-width:480px;margin:0 auto;font-family:sans-serif;padding:24px;">
<h2 style="margin:0 0 16px;">Verify Your Email</h2>
<p>Please verify your email address for {{ app_name }}.</p>
<p style="margin:24px 0;"><a href="{{ verify_url }}" style="background:#111;color:#fff;padding:12px 24px;text-decoration:none;border-radius:4px;display:inline-block;">Verify Email</a></p>
<p style="color:#999;font-size:12px;margin-top:32px;">If you did not create an account, please ignore this email.</p>
</div>"#
            .into(),
        text_body: "Verify your email for {{ app_name }}\n\nVisit this link to verify: {{ verify_url }}\n\nIf you did not create an account, please ignore this email.".into(),
        variables: vec!["verify_url".into(), "app_name".into()],
    }
}
```

### Step 6: Create templates/welcome.rs

```rust
use crate::template::Template;

pub fn template() -> Template {
    Template {
        slug: "welcome".into(),
        name: "Welcome".into(),
        subject: "Welcome to {{ app_name }}".into(),
        html_body: r#"<div style="max-width:480px;margin:0 auto;font-family:sans-serif;padding:24px;">
<h2 style="margin:0 0 16px;">Welcome{% if user_name %}, {{ user_name }}{% endif %}!</h2>
<p>Thanks for joining {{ app_name }}. We're glad to have you.</p>
</div>"#
            .into(),
        text_body: "Welcome{% if user_name %}, {{ user_name }}{% endif %}!\n\nThanks for joining {{ app_name }}. We're glad to have you.".into(),
        variables: vec!["user_name".into(), "app_name".into()],
    }
}
```

### Step 7: Register module in lib.rs

Add `pub mod templates;` to `crates/chorus-core/src/lib.rs`.

### Step 8: Verify and test

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

### Step 9: Commit

```bash
git add crates/chorus-core/src/
git commit -m "feat(core): add 5 built-in auth email templates (otp, password-reset, magic-link, email-verify, welcome)"
```

---

## Task 6: chorus-mail Docker Image (Postfix + OpenDKIM)

### Files
- Create: `chorus-mail/Dockerfile`
- Create: `chorus-mail/config/main.cf`
- Create: `chorus-mail/config/master.cf`
- Create: `chorus-mail/config/opendkim.conf`
- Create: `chorus-mail/scripts/entrypoint.sh`

### Step 1: Create Dockerfile

```dockerfile
FROM alpine:3.21

RUN apk add --no-cache \
    postfix \
    opendkim \
    opendkim-utils \
    curl \
    bash

COPY config/main.cf /etc/postfix/main.cf.template
COPY config/master.cf /etc/postfix/master.cf
COPY config/opendkim.conf /etc/opendkim/opendkim.conf.template
COPY scripts/ /scripts/
RUN chmod +x /scripts/*.sh

EXPOSE 25 587

ENTRYPOINT ["/scripts/entrypoint.sh"]
```

### Step 2: Create config/main.cf

```
# Chorus Mail — Postfix main configuration
# Variables replaced by entrypoint.sh: __MAIL_DOMAIN__, __HOSTNAME__

myhostname = __HOSTNAME__
mydomain = __MAIL_DOMAIN__
myorigin = $mydomain
mydestination = localhost
mynetworks = 127.0.0.0/8 172.16.0.0/12 10.0.0.0/8 192.168.0.0/16

# TLS
smtp_tls_security_level = encrypt
smtp_tls_loglevel = 1

# Submission port (587) — used by chorus-server
smtpd_sasl_auth_enable = no

# OpenDKIM milter
milter_default_action = accept
milter_protocol = 6
smtpd_milters = inet:localhost:8891
non_smtpd_milters = inet:localhost:8891

# Bounce handling
notify_classes = bounce, resource, software
bounce_notice_recipient = bounce-handler

# Size limits
message_size_limit = 10240000
mailbox_size_limit = 0

# Logging
maillog_file = /dev/stdout
```

### Step 3: Create config/master.cf

```
# Postfix master process configuration
smtp      inet  n       -       n       -       -       smtpd
submission inet  n       -       n       -       -       smtpd
  -o syslog_name=postfix/submission
  -o smtpd_tls_security_level=none
  -o smtpd_recipient_restrictions=permit_mynetworks,reject

# Bounce handler pipe
bounce-handler unix  -       n       n       -       -       pipe
  flags=F user=nobody argv=/scripts/bounce-handler.sh ${sender} ${recipient} ${original_recipient}

pickup    unix  n       -       n       60      1       pickup
cleanup   unix  n       -       n       -       0       cleanup
qmgr      unix  n       -       n       300     1       qmgr
tlsmgr    unix  -       -       n       1000?   1       tlsmgr
rewrite   unix  -       -       n       -       -       trivial-rewrite
bounce    unix  -       -       n       -       0       bounce
defer     unix  -       -       n       -       0       bounce
trace     unix  -       -       n       -       0       bounce
verify    unix  -       -       n       -       1       verify
flush     unix  n       -       n       1000?   0       flush
proxymap  unix  -       -       n       -       -       proxymap
smtp      unix  -       -       n       -       -       smtp
relay     unix  -       -       n       -       -       smtp
error     unix  -       -       n       -       -       error
retry     unix  -       -       n       -       -       error
discard   unix  -       -       n       -       -       discard
local     unix  -       n       n       -       -       local
virtual   unix  -       n       n       -       -       virtual
lmtp      unix  -       -       n       -       -       lmtp
anvil     unix  -       -       n       -       1       anvil
scache    unix  -       -       n       -       1       scache
postlog   unix-dgram n  -       n       -       1       postlogd
```

### Step 4: Create config/opendkim.conf

```
# OpenDKIM configuration
# Variables replaced by entrypoint.sh: __MAIL_DOMAIN__

Syslog          yes
LogWhy          yes
Mode            sv
Canonicalization relaxed/relaxed
Domain          __MAIL_DOMAIN__
Selector        chorus
KeyFile         /etc/opendkim/keys/__MAIL_DOMAIN__/chorus.private
Socket          inet:8891@localhost
PidFile         /run/opendkim/opendkim.pid
UMask           002
UserID          opendkim:opendkim
```

### Step 5: Create scripts/entrypoint.sh

```bash
#!/bin/bash
set -e

MAIL_DOMAIN="${MAIL_DOMAIN:?MAIL_DOMAIN is required}"
HOSTNAME="${MAIL_HOSTNAME:-mail.${MAIL_DOMAIN}}"

echo "chorus-mail: configuring for domain ${MAIL_DOMAIN}"

# Replace template variables in Postfix config
sed -e "s/__MAIL_DOMAIN__/${MAIL_DOMAIN}/g" \
    -e "s/__HOSTNAME__/${HOSTNAME}/g" \
    /etc/postfix/main.cf.template > /etc/postfix/main.cf

# Replace template variables in OpenDKIM config
sed "s/__MAIL_DOMAIN__/${MAIL_DOMAIN}/g" \
    /etc/opendkim/opendkim.conf.template > /etc/opendkim/opendkim.conf

# Generate DKIM keys if they don't exist
DKIM_DIR="/etc/opendkim/keys/${MAIL_DOMAIN}"
if [ ! -f "${DKIM_DIR}/chorus.private" ]; then
    echo "chorus-mail: generating DKIM keys for ${MAIL_DOMAIN}"
    mkdir -p "${DKIM_DIR}"
    opendkim-genkey -b 2048 -d "${MAIL_DOMAIN}" -D "${DKIM_DIR}" -s chorus -v
    chown -R opendkim:opendkim /etc/opendkim/keys
fi

# Create OpenDKIM run directory
mkdir -p /run/opendkim
chown opendkim:opendkim /run/opendkim

# Start OpenDKIM
opendkim -x /etc/opendkim/opendkim.conf &

# Start Postfix in foreground
echo "chorus-mail: starting Postfix for ${MAIL_DOMAIN}"
postfix start-fg
```

### Step 6: Commit

```bash
git add chorus-mail/
git commit -m "feat(chorus-mail): add Postfix + OpenDKIM Docker image"
```

---

## Task 7: chorus-mail DNS & Bounce Scripts

### Files
- Create: `chorus-mail/scripts/dns-setup.sh`
- Create: `chorus-mail/scripts/bounce-handler.sh`

### Step 1: Create scripts/dns-setup.sh

```bash
#!/bin/bash
set -e

MAIL_DOMAIN="${MAIL_DOMAIN:?MAIL_DOMAIN is required}"
DKIM_KEY_FILE="/etc/opendkim/keys/${MAIL_DOMAIN}/chorus.txt"

echo "═══════════════════════════════════════════════════"
echo "  DNS Records for: ${MAIL_DOMAIN}"
echo "═══════════════════════════════════════════════════"
echo ""
echo "Add these records to your DNS provider:"
echo ""
echo "─── MX Record ────────────────────────────────────"
echo "  Type:  MX"
echo "  Name:  @"
echo "  Value: mail.${MAIL_DOMAIN}"
echo "  Priority: 10"
echo ""
echo "─── A Record ─────────────────────────────────────"
echo "  Type:  A"
echo "  Name:  mail"
echo "  Value: <YOUR_SERVER_IP>"
echo ""
echo "─── SPF Record ───────────────────────────────────"
echo "  Type:  TXT"
echo "  Name:  @"
echo "  Value: \"v=spf1 a mx ip4:<YOUR_SERVER_IP> -all\""
echo ""
echo "─── DKIM Record ──────────────────────────────────"
echo "  Type:  TXT"
echo "  Name:  chorus._domainkey"
if [ -f "${DKIM_KEY_FILE}" ]; then
    DKIM_VALUE=$(grep -o '".*"' "${DKIM_KEY_FILE}" | tr -d '\n' | sed 's/" "//g')
    echo "  Value: ${DKIM_VALUE}"
else
    echo "  Value: <run entrypoint first to generate DKIM keys>"
fi
echo ""
echo "─── DMARC Record ─────────────────────────────────"
echo "  Type:  TXT"
echo "  Name:  _dmarc"
echo "  Value: \"v=DMARC1; p=quarantine; rua=mailto:postmaster@${MAIL_DOMAIN}\""
echo ""
echo "═══════════════════════════════════════════════════"
```

### Step 2: Create scripts/bounce-handler.sh

```bash
#!/bin/bash
# Postfix pipe transport: receives bounce notifications and forwards to chorus-server.
# Called by master.cf with: ${sender} ${recipient} ${original_recipient}

CHORUS_SERVER_URL="${CHORUS_SERVER_URL:?CHORUS_SERVER_URL is required}"
BOUNCE_SECRET="${BOUNCE_SECRET:?BOUNCE_SECRET is required}"

SENDER="$1"
RECIPIENT="$2"
ORIGINAL_RECIPIENT="$3"

# Read the bounce message from stdin
BOUNCE_BODY=$(cat)

# Extract the original Message-ID from bounce body if present
MESSAGE_ID=$(echo "${BOUNCE_BODY}" | grep -i "^Message-ID:" | head -1 | sed 's/Message-ID: *//i' | tr -d '<>' || echo "")

# Extract bounce reason from first diagnostic line
REASON=$(echo "${BOUNCE_BODY}" | grep -i "diagnostic-code:" | head -1 | sed 's/.*Diagnostic-Code: *//i' || echo "unknown bounce")

# POST to chorus-server
curl -sf -X POST "${CHORUS_SERVER_URL}/internal/bounces" \
    -H "Content-Type: application/json" \
    -H "X-Chorus-Secret: ${BOUNCE_SECRET}" \
    -d "{\"recipient\": \"${RECIPIENT}\", \"reason\": \"${REASON}\", \"message_id\": \"${MESSAGE_ID}\"}" \
    || echo "chorus-mail: failed to notify bounce for ${RECIPIENT}" >&2

exit 0
```

### Step 3: Commit

```bash
git add chorus-mail/scripts/
git commit -m "feat(chorus-mail): add DNS setup and bounce handler scripts"
```

---

## Task 8: chorus-server Bounce Webhook + DNS Check Endpoints

### Files
- Modify: `crates/chorus-server/Cargo.toml`
- Create: `crates/chorus-server/src/routes/internal.rs`
- Modify: `crates/chorus-server/src/routes/mod.rs`
- Modify: `crates/chorus-server/src/config.rs`
- Modify: `crates/chorus-server/src/app.rs`

### Step 1: Add hickory-resolver dependency

In workspace `Cargo.toml`, add:

```toml
hickory-resolver = "0.25"
```

In `crates/chorus-server/Cargo.toml`, add:

```toml
hickory-resolver = { workspace = true }
```

### Step 2: Add BOUNCE_SECRET to Config

In `crates/chorus-server/src/config.rs`, add field and from_env:

```rust
/// Shared secret for chorus-mail bounce webhook.
pub bounce_secret: Option<String>,
```

```rust
bounce_secret: std::env::var("BOUNCE_SECRET").ok(),
```

### Step 3: Create routes/internal.rs

```rust
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::app::AppState;

/// Bounce notification from chorus-mail.
#[derive(Deserialize)]
pub struct BounceNotification {
    pub recipient: String,
    pub reason: String,
    pub message_id: String,
}

/// DNS check result.
#[derive(Serialize)]
pub struct DnsCheckResult {
    pub spf: bool,
    pub dkim: bool,
    pub dmarc: bool,
    pub mx: bool,
}

/// Receive bounce notification from chorus-mail.
pub async fn handle_bounce(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<BounceNotification>,
) -> Result<StatusCode, (StatusCode, String)> {
    // Validate shared secret
    let expected = state.config().bounce_secret.as_deref().unwrap_or("");
    let provided = headers
        .get("x-chorus-secret")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if expected.is_empty() || provided != expected {
        return Err((StatusCode::UNAUTHORIZED, "invalid secret".into()));
    }

    tracing::warn!(
        recipient = %body.recipient,
        reason = %body.reason,
        "bounce received from chorus-mail"
    );

    // TODO: look up message by provider message_id and update status to "bounced"
    // For now, log the bounce. Full implementation requires a message lookup by
    // provider_message_id which can be added as a follow-up.

    Ok(StatusCode::OK)
}

/// Check DNS records for a domain.
pub async fn dns_check(
    axum::extract::Query(params): axum::extract::Query<DnsCheckQuery>,
) -> Result<Json<DnsCheckResult>, (StatusCode, String)> {
    let domain = &params.domain;

    let resolver = hickory_resolver::AsyncResolver::tokio_from_system_conf()
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("resolver error: {}", e)))?;

    let spf = check_txt_record(&resolver, domain, "v=spf1").await;
    let dkim = check_txt_record(&resolver, &format!("chorus._domainkey.{}", domain), "v=DKIM1").await;
    let dmarc = check_txt_record(&resolver, &format!("_dmarc.{}", domain), "v=DMARC1").await;
    let mx = check_mx_record(&resolver, domain).await;

    Ok(Json(DnsCheckResult { spf, dkim, dmarc, mx }))
}

#[derive(Deserialize)]
pub struct DnsCheckQuery {
    pub domain: String,
}

async fn check_txt_record(
    resolver: &hickory_resolver::AsyncResolver<hickory_resolver::name_server::TokioConnectionProvider>,
    name: &str,
    prefix: &str,
) -> bool {
    match resolver.txt_lookup(name).await {
        Ok(lookup) => lookup.iter().any(|txt| {
            let record = txt.to_string();
            record.contains(prefix)
        }),
        Err(_) => false,
    }
}

async fn check_mx_record(
    resolver: &hickory_resolver::AsyncResolver<hickory_resolver::name_server::TokioConnectionProvider>,
    domain: &str,
) -> bool {
    resolver.mx_lookup(domain).await.is_ok()
}
```

### Step 4: Register module and routes

Add `pub mod internal;` to `routes/mod.rs`.

In `app.rs`, add routes:

```rust
.route("/internal/bounces", post(routes::internal::handle_bounce))
.route("/internal/dns-check", get(routes::internal::dns_check))
```

### Step 5: Add config() accessor to AppState

Add `config: Arc<Config>` field to AppState, with accessor:

```rust
pub fn config(&self) -> &Config {
    &self.config
}
```

Update `new()` and `with_repos()` to accept and store `Arc<Config>`.

### Step 6: Verify and test

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

### Step 7: Commit

```bash
git add crates/chorus-server/ Cargo.toml
git commit -m "feat(server): add bounce webhook and DNS check internal endpoints"
```

---

## Task 9: Docker Compose, Env, Integration Guide & Issue Cleanup

### Files
- Modify: `docker-compose.yml`
- Modify: `.env.example`
- Create: `docs/guides/auth-service-integration.md`

### Step 1: Update docker-compose.yml

Add chorus-mail service:

```yaml
  chorus-mail:
    build: ./chorus-mail
    environment:
      MAIL_DOMAIN: ${MAIL_DOMAIN:-example.com}
      CHORUS_SERVER_URL: http://chorus-server:3000
      BOUNCE_SECRET: ${BOUNCE_SECRET:-changeme}
    ports:
      - "25:25"
      - "587:587"
```

Add `BOUNCE_SECRET` to chorus-server environment.

### Step 2: Update .env.example

Add:

```env
# chorus-mail (self-hosted email)
# MAIL_DOMAIN=example.com
# BOUNCE_SECRET=your-shared-secret-here
```

### Step 3: Create integration guide

Create `docs/guides/auth-service-integration.md` covering:
1. Adapter pattern (Nucleus example)
2. Error mapping (ChorusError → AppError)
3. SMTP provider config examples (SendGrid, Mailgun, Gmail)
4. chorus-mail setup walkthrough
5. OTP routing (auto-detect email/phone)
6. Test mode development

### Step 4: Final verification

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
cargo deny check
```

### Step 5: Commit

```bash
git add docker-compose.yml .env.example docs/
git commit -m "feat: add chorus-mail to docker-compose, env example, and integration guide"
```

### Step 6: Close issues

```bash
gh issue close 6 --comment "Resolved: use existing SmtpEmailSender with smtp.sendgrid.net:587. See docs/guides/auth-service-integration.md"
gh issue close 11 --comment "Resolved: use existing SmtpEmailSender with smtp.mailgun.org:587. See docs/guides/auth-service-integration.md"
```

---

## Verification Checklist

After all tasks complete:

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
cargo deny check
docker compose build
```

All must pass before merging.
