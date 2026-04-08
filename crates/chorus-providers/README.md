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
