# Auth Service Integration Guide

How to integrate Chorus into your authentication service for sending OTP codes, password resets, magic links, and email verification.

## Adapter Pattern (Rust — Nucleus Example)

Wrap Chorus in a thin adapter so your auth service depends on a local trait, not Chorus directly:

```rust
use chorus::client::Chorus;
use chorus::types::{SmsMessage, TemplateEmailMessage};
use std::collections::HashMap;
use std::sync::Arc;

pub struct NotificationService {
    chorus: Arc<Chorus>,
    app_name: String,
}

impl NotificationService {
    pub fn new(chorus: Arc<Chorus>, app_name: String) -> Self {
        Self { chorus, app_name }
    }

    pub async fn send_otp(&self, to_email: &str, code: &str) -> Result<(), AppError> {
        let vars = HashMap::from([
            ("code".into(), code.into()),
            ("app_name".into(), self.app_name.clone()),
            ("expiry".into(), "10 minutes".into()),
        ]);
        self.chorus
            .send_template_email(&TemplateEmailMessage {
                to: to_email.into(),
                from: None,
                template_slug: "otp".into(),
                variables: vars,
            })
            .await
            .map_err(AppError::from)?;
        Ok(())
    }
}
```

## Error Mapping

Map `ChorusError` to your app's error type:

```rust
impl From<ChorusError> for AppError {
    fn from(err: ChorusError) -> Self {
        match err {
            ChorusError::Validation(msg) => AppError::BadRequest(msg),
            ChorusError::RateLimited { retry_after_secs } => {
                AppError::TooManyRequests(retry_after_secs)
            }
            _ => AppError::Internal(err.to_string()),
        }
    }
}
```

## REST API (Java — Orbit Example)

For non-Rust services, use the Chorus REST API:

```java
// POST /v1/email/send
HttpRequest request = HttpRequest.newBuilder()
    .uri(URI.create("http://chorus:3000/v1/email/send"))
    .header("Authorization", "Bearer ch_live_xxx")
    .header("Content-Type", "application/json")
    .POST(HttpRequest.BodyPublishers.ofString("""
        {
            "to": "user@example.com",
            "template_slug": "password-reset",
            "variables": {
                "app_name": "Orbit",
                "reset_url": "https://orbit.app/reset?token=abc",
                "expiry": "1 hour"
            }
        }
    """))
    .build();
```

## SMTP Provider Configuration

### SendGrid via SMTP

```env
SMTP_HOST=smtp.sendgrid.net
SMTP_PORT=587
SMTP_USERNAME=apikey
SMTP_PASSWORD=SG.your-api-key
FROM_EMAIL=noreply@yourdomain.com
```

### Mailgun via SMTP

```env
SMTP_HOST=smtp.mailgun.org
SMTP_PORT=587
SMTP_USERNAME=postmaster@yourdomain.com
SMTP_PASSWORD=your-mailgun-password
FROM_EMAIL=noreply@yourdomain.com
```

### Gmail via SMTP (dev only)

```env
SMTP_HOST=smtp.gmail.com
SMTP_PORT=587
SMTP_USERNAME=you@gmail.com
SMTP_PASSWORD=your-app-password
FROM_EMAIL=you@gmail.com
```

## chorus-mail (Self-Hosted)

For zero-cost email delivery, use the built-in chorus-mail Docker image:

```bash
# 1. Start the stack
MAIL_DOMAIN=yourdomain.com docker compose up -d

# 2. Print required DNS records
docker compose exec chorus-mail /scripts/dns-setup.sh

# 3. Add the printed DNS records (MX, SPF, DKIM, DMARC) to your DNS provider

# 4. Configure chorus-server to use local SMTP
SMTP_HOST=chorus-mail
SMTP_PORT=587
FROM_EMAIL=noreply@yourdomain.com
```

## OTP Routing (Auto-detect Email/Phone)

Chorus can auto-route OTP delivery based on the recipient format:

```rust
// Email detected → uses email template
chorus.send_otp("user@example.com", "123456").await?;

// Phone detected → sends SMS
chorus.send_otp("+66812345678", "123456").await?;
```

## Test Mode

Use `ch_test_` prefixed API keys during development. Test mode logs messages without sending them to real providers:

```env
# In development
CHORUS_API_KEY=ch_test_abc123

# In production
CHORUS_API_KEY=ch_live_abc123
```

## Built-in Templates

Chorus ships with 5 auth templates ready to use:

| Slug | Variables | Use Case |
|------|-----------|----------|
| `otp` | `code`, `app_name`, `expiry` | 2FA / verification codes |
| `password-reset` | `reset_url`, `app_name`, `expiry` | Password reset links |
| `magic-link` | `magic_url`, `app_name`, `expiry` | Passwordless sign-in |
| `email-verify` | `verify_url`, `app_name` | New account email verification |
| `welcome` | `user_name`, `app_name` | Post-signup welcome email |

All templates support Jinja2 syntax (`{% if %}`, `{% for %}`, `{{ var | default('x') }}`).
