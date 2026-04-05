use crate::error::ChorusError;
use crate::types::{DeliveryStatus, SendResult, SmsMessage};
use async_trait::async_trait;

/// Trait for SMS delivery providers.
///
/// Implement this trait to add a new SMS provider to Chorus.
/// Providers are used by [`WaterfallRouter`](crate::router::WaterfallRouter) for
/// multi-provider failover.
#[async_trait]
pub trait SmsSender: Send + Sync {
    /// Returns the provider name (e.g., `"twilio"`, `"telnyx"`).
    fn provider_name(&self) -> &str;
    /// Sends an SMS message and returns the delivery result.
    async fn send(&self, msg: &SmsMessage) -> Result<SendResult, ChorusError>;
    /// Checks the delivery status of a previously sent message by ID.
    async fn check_status(&self, message_id: &str) -> Result<DeliveryStatus, ChorusError>;
}
