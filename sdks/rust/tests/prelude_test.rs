use chorus::prelude::*;

#[test]
fn prelude_imports_all_key_types() {
    let _msg = SmsMessage {
        to: "+1234567890".into(),
        body: "test".into(),
        from: None,
    };

    let _email = EmailMessage {
        to: "test@example.com".into(),
        subject: "Test".into(),
        html_body: "<p>Hi</p>".into(),
        text_body: "Hi".into(),
        from: None,
    };

    let _chorus = Chorus::builder().build();
    let _router = WaterfallRouter::new();
}

#[test]
fn module_re_exports_work() {
    let _: chorus::types::Channel = chorus::types::Channel::Sms;
}

#[test]
fn providers_re_exported() {
    let _sender = chorus::providers::sms::telnyx::TelnyxSmsSender::new(
        "test-key".into(),
        Some("+1234567890".into()),
    );
}
