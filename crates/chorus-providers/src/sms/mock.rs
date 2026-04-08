use async_trait::async_trait;
use chorus::error::ChorusError;
use chorus::sms::SmsSender;
use chorus::types::{Channel, DeliveryStatus, SendResult, SmsMessage};
use chrono::Utc;
use uuid::Uuid;

/// Mock SMS provider that logs messages instead of sending.
/// Used for development and testing.
pub struct MockSmsSender;

#[async_trait]
impl SmsSender for MockSmsSender {
    fn provider_name(&self) -> &str {
        "mock"
    }

    async fn send(&self, msg: &SmsMessage) -> Result<SendResult, ChorusError> {
        tracing::info!(
            provider = "mock",
            to = %msg.to,
            body_len = msg.body.len(),
            "SMS would be sent (mock mode)"
        );

        Ok(SendResult {
            message_id: Uuid::new_v4().to_string(),
            provider: "mock".to_string(),
            channel: Channel::Sms,
            status: DeliveryStatus::Sent,
            created_at: Utc::now(),
        })
    }

    async fn check_status(&self, _message_id: &str) -> Result<DeliveryStatus, ChorusError> {
        Ok(DeliveryStatus::Delivered)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn mock_sms_send_succeeds() {
        let sender = MockSmsSender;
        let msg = SmsMessage {
            to: "+66812345678".to_string(),
            body: "Test OTP: 123456".to_string(),
            from: None,
        };
        let result = sender.send(&msg).await.unwrap();
        assert_eq!(result.provider, "mock");
        assert_eq!(result.channel, Channel::Sms);
        assert!(matches!(result.status, DeliveryStatus::Sent));
    }

    #[tokio::test]
    async fn mock_sms_check_status_returns_delivered() {
        let sender = MockSmsSender;
        let status = sender.check_status("any-id").await.unwrap();
        assert_eq!(status, DeliveryStatus::Delivered);
    }
}
