use crate::email::EmailSender;
use crate::error::ChorusError;
use crate::sms::SmsSender;
use crate::types::{Channel, EmailMessage, SendResult, SmsMessage};
use std::sync::Arc;

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
                        continue;
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
                        continue;
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

    struct SuccessSms;
    #[async_trait::async_trait]
    impl SmsSender for SuccessSms {
        fn provider_name(&self) -> &str {
            "test-sms"
        }
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
        fn provider_name(&self) -> &str {
            "fail-sms"
        }
        async fn send(&self, _msg: &SmsMessage) -> Result<SendResult, ChorusError> {
            Err(ChorusError::Provider {
                provider: "fail-sms".into(),
                message: "timeout".into(),
            })
        }
        async fn check_status(&self, _id: &str) -> Result<DeliveryStatus, ChorusError> {
            Ok(DeliveryStatus::Failed {
                reason: "timeout".into(),
            })
        }
    }

    struct SuccessEmail;
    #[async_trait::async_trait]
    impl EmailSender for SuccessEmail {
        fn provider_name(&self) -> &str {
            "test-email"
        }
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

        let result = router
            .send_otp("user@test.com", "123456", "TestApp")
            .await
            .unwrap();
        assert_eq!(result.channel, Channel::Email);
        assert_eq!(result.provider, "test-email");
    }

    #[tokio::test]
    async fn waterfall_sends_sms_for_phone_recipient() {
        let router = WaterfallRouter::new()
            .add_email(Arc::new(SuccessEmail))
            .add_sms(Arc::new(SuccessSms));

        let result = router
            .send_otp("+66812345678", "123456", "TestApp")
            .await
            .unwrap();
        assert_eq!(result.channel, Channel::Sms);
        assert_eq!(result.provider, "test-sms");
    }

    #[tokio::test]
    async fn waterfall_fallback_on_failure() {
        let router = WaterfallRouter::new()
            .add_sms(Arc::new(FailSms))
            .add_sms(Arc::new(SuccessSms));

        let result = router
            .send_otp("+66812345678", "123456", "TestApp")
            .await
            .unwrap();
        assert_eq!(result.provider, "test-sms");
    }

    #[tokio::test]
    async fn waterfall_all_fail_returns_error() {
        let router = WaterfallRouter::new().add_sms(Arc::new(FailSms));

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

        let msg = SmsMessage {
            to: "+66812345678".into(),
            body: "Hi".into(),
            from: None,
        };
        let result = router.send_sms(&msg).await.unwrap();
        assert_eq!(result.channel, Channel::Sms);
    }

    #[tokio::test]
    async fn send_email_directly() {
        let router = WaterfallRouter::new()
            .add_email(Arc::new(SuccessEmail))
            .add_sms(Arc::new(SuccessSms));

        let msg = EmailMessage {
            to: "user@test.com".into(),
            subject: "Hi".into(),
            html_body: "<p>Hi</p>".into(),
            text_body: "Hi".into(),
            from: None,
        };
        let result = router.send_email(&msg).await.unwrap();
        assert_eq!(result.channel, Channel::Email);
    }

    #[tokio::test]
    async fn send_sms_no_sms_providers_returns_error() {
        let router = WaterfallRouter::new().add_email(Arc::new(SuccessEmail));

        let msg = SmsMessage {
            to: "+66812345678".into(),
            body: "Hi".into(),
            from: None,
        };
        let result = router.send_sms(&msg).await;
        assert!(matches!(result, Err(ChorusError::AllProvidersFailed)));
    }

    #[tokio::test]
    async fn send_email_no_email_providers_returns_error() {
        let router = WaterfallRouter::new().add_sms(Arc::new(SuccessSms));

        let msg = EmailMessage {
            to: "user@test.com".into(),
            subject: "Hi".into(),
            html_body: "<p>Hi</p>".into(),
            text_body: "Hi".into(),
            from: None,
        };
        let result = router.send_email(&msg).await;
        assert!(matches!(result, Err(ChorusError::AllProvidersFailed)));
    }

    #[tokio::test]
    async fn send_sms_failover_across_providers() {
        let router = WaterfallRouter::new()
            .add_sms(Arc::new(FailSms))
            .add_sms(Arc::new(SuccessSms));

        let msg = SmsMessage {
            to: "+66812345678".into(),
            body: "Hi".into(),
            from: None,
        };
        let result = router.send_sms(&msg).await.unwrap();
        assert_eq!(result.provider, "test-sms");
    }

    #[tokio::test]
    async fn send_sms_all_fail_returns_error() {
        let router = WaterfallRouter::new()
            .add_sms(Arc::new(FailSms))
            .add_sms(Arc::new(FailSms));

        let msg = SmsMessage {
            to: "+66812345678".into(),
            body: "Hi".into(),
            from: None,
        };
        let result = router.send_sms(&msg).await;
        assert!(matches!(result, Err(ChorusError::AllProvidersFailed)));
    }
}
