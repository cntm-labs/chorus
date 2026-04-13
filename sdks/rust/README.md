# chorus-rs

Official Rust SDK for [Chorus](https://github.com/cntm-labs/chorus) — open-source CPaaS with SMS, Email, and OTP.

Re-exports `chorus-core` (types, traits, routing) and `chorus-providers` (Telnyx, Twilio, Plivo, Resend, SES, Mailgun, SMTP) so you only need one dependency.

## Quick Start

```rust
use chorus::prelude::*;
use chorus::providers::sms::telnyx::TelnyxSmsSender;
use std::sync::Arc;

let telnyx = TelnyxSmsSender::new("api-key".into(), Some("+1234567890".into()));

let chorus = Chorus::builder()
    .add_sms_provider(Arc::new(telnyx))
    .default_from_sms("+1234567890".into())
    .build();

let msg = SmsMessage {
    to: "+0987654321".into(),
    body: "Hello from Chorus!".into(),
    from: None,
};
chorus.send_sms(&msg).await?;
```

## License

[MIT](../../LICENSE)
