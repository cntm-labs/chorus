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
