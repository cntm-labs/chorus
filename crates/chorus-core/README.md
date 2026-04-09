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
