pub mod dead_letter;
pub mod delayed;
pub mod enqueue;
pub mod router_builder;
pub mod webhook_dispatch;
pub mod worker;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Main work queue key.
pub const QUEUE_KEY: &str = "chorus:jobs";
/// Delayed retry queue (sorted set, score = retry-at Unix timestamp).
pub const DELAYED_KEY: &str = "chorus:delayed";
/// Dead letter queue for permanently failed jobs.
pub const DEAD_LETTER_KEY: &str = "chorus:dead_letters";
/// Maximum delivery attempts before moving to DLQ.
pub const MAX_RETRIES: i32 = 3;

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
    /// Current attempt number (0-based, incremented on retry).
    pub attempt: i32,
}
