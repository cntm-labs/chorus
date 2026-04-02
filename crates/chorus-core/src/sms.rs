use crate::error::ChorusError;
use crate::types::{DeliveryStatus, SendResult, SmsMessage};
use async_trait::async_trait;

#[async_trait]
pub trait SmsSender: Send + Sync {
    fn provider_name(&self) -> &str;
    async fn send(&self, msg: &SmsMessage) -> Result<SendResult, ChorusError>;
    async fn check_status(&self, message_id: &str) -> Result<DeliveryStatus, ChorusError>;
}
