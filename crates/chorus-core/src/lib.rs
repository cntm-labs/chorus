//! # chorus-core
//!
//! Core traits, types, and routing engine for Chorus CPaaS.
//!
//! Chorus provides SMS and Email delivery with smart routing, multi-provider
//! failover, and cost optimization through waterfall routing (email-first,
//! SMS-fallback).
//!
//! ## Key Components
//!
//! - [`Chorus`](client::Chorus) — Main client with builder pattern
//! - [`SmsSender`](sms::SmsSender) / [`EmailSender`](email::EmailSender) — Provider traits
//! - [`WaterfallRouter`](router::WaterfallRouter) — Cost-optimized routing engine
//! - [`Template`](template::Template) — `{{variable}}` template rendering
//!
//! ## Quick Start
//!
//! ```rust,no_run
//! use chorus_core::client::Chorus;
//! use chorus_core::types::SmsMessage;
//! use std::sync::Arc;
//!
//! # async fn example() -> Result<(), chorus_core::error::ChorusError> {
//! let chorus = Chorus::builder()
//!     // .add_sms_provider(Arc::new(my_provider))
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

pub mod client;
pub mod email;
pub mod error;
pub mod router;
pub mod sms;
pub mod template;
pub mod templates;
pub mod types;
