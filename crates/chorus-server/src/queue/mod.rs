pub mod enqueue;
pub mod router_builder;
pub mod worker;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A job representing a message to be sent.
#[derive(Debug, Serialize, Deserialize)]
pub struct SendJob {
    /// The message ID from the database.
    pub message_id: Uuid,
    /// The account that owns this message.
    pub account_id: Uuid,
    /// `"sms"` or `"email"`.
    pub channel: String,
    /// `"live"` or `"test"`.
    pub environment: String,
}
