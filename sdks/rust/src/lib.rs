//! # chorus-sdk
//!
//! Official Rust SDK for Chorus — open-source CPaaS with SMS, Email, and OTP.
//!
//! This crate re-exports [`chorus-core`] for convenience. Use the [`prelude`]
//! module to import commonly used types.
//!
//! ## Quick Start
//!
//! ```rust,no_run
//! use chorus_sdk::prelude::*;
//! use std::sync::Arc;
//!
//! # async fn example() -> Result<(), ChorusError> {
//! let chorus = Chorus::builder()
//!     .default_from_sms("+1234567890".into())
//!     .build();
//!
//! let msg = SmsMessage {
//!     to: "+0987654321".into(),
//!     body: "Hello from Chorus!".into(),
//!     from: None,
//! };
//! let result = chorus.send_sms(&msg).await?;
//! # Ok(())
//! # }
//! ```

// Re-export all public modules from chorus-core.
pub use chorus::client;
pub use chorus::email;
pub use chorus::error;
pub use chorus::router;
pub use chorus::sms;
pub use chorus::template;
pub use chorus::templates;
pub use chorus::types;

/// Prelude module — import commonly used types with `use chorus_sdk::prelude::*`.
pub mod prelude {
    pub use chorus::client::Chorus;
    pub use chorus::email::EmailSender;
    pub use chorus::error::ChorusError;
    pub use chorus::router::WaterfallRouter;
    pub use chorus::sms::SmsSender;
    pub use chorus::types::{
        Channel, DeliveryStatus, EmailMessage, SendResult, SmsMessage, TemplateEmailMessage,
    };
}
