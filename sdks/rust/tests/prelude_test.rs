use chorus_sdk::prelude::*;

#[test]
fn prelude_imports_all_key_types() {
    // Verify all prelude types are accessible
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

    // Builder is accessible
    let _chorus = Chorus::builder().build();

    // Router is accessible
    let _router = WaterfallRouter::new();
}

#[test]
fn module_re_exports_work() {
    // Direct module access works
    let _: chorus_sdk::types::Channel = chorus_sdk::types::Channel::Sms;
}
