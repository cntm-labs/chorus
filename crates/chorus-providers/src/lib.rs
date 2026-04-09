//! # chorus-providers
//!
//! SMS and Email provider implementations for Chorus CPaaS.
//!
//! ## SMS Providers
//!
//! | Provider | Struct | API |
//! |----------|--------|-----|
//! | Telnyx | [`sms::telnyx::TelnyxSmsSender`] | REST |
//! | Twilio | [`sms::twilio::TwilioSmsSender`] | REST |
//! | Plivo | [`sms::plivo::PlivoSmsSender`] | REST |
//! | Mock | [`sms::mock::MockSmsSender`] | In-memory (testing) |
//!
//! ## Email Providers
//!
//! | Provider | Struct | API |
//! |----------|--------|-----|
//! | Resend | [`email::resend::ResendEmailSender`] | REST |
//! | AWS SES | [`email::ses::SesEmailSender`] | SMTP |
//! | SMTP | [`email::smtp::SmtpEmailSender`] | SMTP |
//! | Mock | [`email::mock::MockEmailSender`] | In-memory (testing) |
//!
//! All providers implement [`chorus::sms::SmsSender`] or
//! [`chorus::email::EmailSender`] and can be used interchangeably
//! with [`chorus::router::WaterfallRouter`].

pub mod email;
pub mod sms;
