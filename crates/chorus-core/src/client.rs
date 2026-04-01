use crate::email::EmailSender;
use crate::error::ChorusError;
use crate::router::WaterfallRouter;
use crate::sms::SmsSender;
use crate::template::Template;
use crate::types::{EmailMessage, SendResult, SmsMessage};
use std::collections::HashMap;
use std::sync::Arc;

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
        let msg = if msg.from.is_none() && self.default_from_sms.is_some() {
            let mut m = msg.clone();
            m.from = self.default_from_sms.clone();
            std::borrow::Cow::Owned(m)
        } else {
            std::borrow::Cow::Borrowed(msg)
        };
        self.router.send_sms(&msg).await
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

    struct TestEmail;
    #[async_trait::async_trait]
    impl EmailSender for TestEmail {
        fn provider_name(&self) -> &str {
            "test"
        }
        async fn send(&self, _msg: &EmailMessage) -> Result<SendResult, ChorusError> {
            Ok(SendResult {
                message_id: "e1".into(),
                provider: "test".into(),
                channel: Channel::Email,
                status: DeliveryStatus::Sent,
                created_at: chrono::Utc::now(),
            })
        }
    }

    struct TestSms;
    #[async_trait::async_trait]
    impl SmsSender for TestSms {
        fn provider_name(&self) -> &str {
            "test"
        }
        async fn send(&self, _msg: &SmsMessage) -> Result<SendResult, ChorusError> {
            Ok(SendResult {
                message_id: "s1".into(),
                provider: "test".into(),
                channel: Channel::Sms,
                status: DeliveryStatus::Sent,
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
                slug: "otp".into(),
                name: "OTP".into(),
                subject: "Code: {{code}}".into(),
                html_body: "<p>{{code}}</p>".into(),
                text_body: "{{code}}".into(),
                variables: vec!["code".into()],
            })
            .build();

        let mut vars = HashMap::new();
        vars.insert("code".into(), "123456".into());

        let result = chorus
            .send_email_template("user@test.com", "otp", &vars)
            .await
            .unwrap();
        assert_eq!(result.channel, Channel::Email);
    }

    #[tokio::test]
    async fn chorus_template_not_found() {
        let chorus = Chorus::builder()
            .add_email_provider(Arc::new(TestEmail))
            .build();

        let vars = HashMap::new();
        let result = chorus
            .send_email_template("user@test.com", "nonexistent", &vars)
            .await;
        assert!(matches!(result, Err(ChorusError::TemplateNotFound(_))));
    }

    #[tokio::test]
    async fn chorus_send_otp_email() {
        let chorus = Chorus::builder()
            .add_email_provider(Arc::new(TestEmail))
            .add_sms_provider(Arc::new(TestSms))
            .build();

        let result = chorus
            .send_otp("user@test.com", "123456", "App")
            .await
            .unwrap();
        assert_eq!(result.channel, Channel::Email);
    }

    #[tokio::test]
    async fn chorus_send_otp_sms() {
        let chorus = Chorus::builder()
            .add_email_provider(Arc::new(TestEmail))
            .add_sms_provider(Arc::new(TestSms))
            .build();

        let result = chorus
            .send_otp("+66812345678", "123456", "App")
            .await
            .unwrap();
        assert_eq!(result.channel, Channel::Sms);
    }
}
