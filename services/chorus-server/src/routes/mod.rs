pub mod admin;
pub mod batch;
pub mod billing;
pub mod email;
pub mod health;
pub mod internal;
pub mod keys;
pub mod messages;
pub mod otp;
pub mod provider_configs;
pub mod sms;
pub mod suppressions;
pub mod totp;
pub mod verifications;
pub mod webhooks;

use axum::Router;
use std::sync::Arc;

use crate::app::AppState;

/// Build the V1 sub-router with all public endpoints.
pub fn v1_router() -> Router<Arc<AppState>> {
    Router::new()
        .nest("/sms", sms::router())
        .nest("/email", email::router())
        .nest("/messages", messages::router())
        .nest("/keys", keys::router())
        .nest("/otp", otp::router())
        .nest("/providers", provider_configs::router())
        .nest("/webhooks", webhooks::router())
        .nest("/suppressions", suppressions::router())
        .nest("/billing", billing::router())
        .nest("/verifications", verifications::router())
        .nest("/totp", totp::router())
        .merge(batch::router())
}

/// Build the main application router with all namespaces.
pub fn main_router() -> Router<Arc<AppState>> {
    Router::new()
        .nest("/health", health::router())
        .nest("/internal", internal::router())
        .nest("/admin", admin::router())
        .nest("/v1", v1_router())
}


