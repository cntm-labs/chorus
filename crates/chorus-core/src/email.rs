use crate::error::ChorusError;
use crate::types::{EmailMessage, SendResult};
use async_trait::async_trait;

/// Trait for email delivery providers.
///
/// Implement this trait to add a new email provider to Chorus.
/// Providers are used by [`WaterfallRouter`](crate::router::WaterfallRouter) for
/// multi-provider failover.
#[async_trait]
pub trait EmailSender: Send + Sync {
    /// Returns the provider name (e.g., `"resend"`, `"ses"`).
    fn provider_name(&self) -> &str;
    /// Sends an email message and returns the delivery result.
    async fn send(&self, msg: &EmailMessage) -> Result<SendResult, ChorusError>;
}
