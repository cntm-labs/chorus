//! # chorus-rs
//!
//! Official Rust SDK for Chorus — open-source CPaaS with SMS, Email, and OTP.
//!
//! This crate re-exports [`chorus-core`] (types, traits, routing) and
//! [`chorus-providers`] (Telnyx, Twilio, Plivo, Resend, SES, Mailgun, SMTP)
//! so you only need one dependency.
//!
//! ## Quick Start
//!
//! ```rust,no_run
//! use chorus::prelude::*;
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
pub use chorus_core::client;
pub use chorus_core::email;
pub use chorus_core::error;
pub use chorus_core::router;
pub use chorus_core::sms;
pub use chorus_core::template;
pub use chorus_core::templates;
pub use chorus_core::types;

// Re-export provider implementations from chorus-providers.
pub mod providers {
    pub use chorus_providers::email;
    pub use chorus_providers::sms;
}

/// Prelude module — import commonly used types with `use chorus::prelude::*`.
pub mod prelude {
    pub use chorus_core::client::Chorus;
    pub use chorus_core::email::EmailSender;
    pub use chorus_core::error::ChorusError;
    pub use chorus_core::router::WaterfallRouter;
    pub use chorus_core::sms::SmsSender;
    pub use chorus_core::types::{
        Channel, DeliveryStatus, EmailMessage, SendResult, SmsMessage, TemplateEmailMessage,
    };
}
