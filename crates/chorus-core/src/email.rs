use async_trait::async_trait;
use crate::error::ChorusError;
use crate::types::{EmailMessage, SendResult};

#[async_trait]
pub trait EmailSender: Send + Sync {
    fn provider_name(&self) -> &str;
    async fn send(&self, msg: &EmailMessage) -> Result<SendResult, ChorusError>;
}
