use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::app::AppState;

/// Bounce notification from chorus-mail.
#[derive(Deserialize)]
pub struct BounceNotification {
    pub recipient: String,
    pub reason: String,
    pub message_id: String,
}

/// DNS check result.
#[derive(Serialize)]
pub struct DnsCheckResult {
    pub spf: bool,
    pub dkim: bool,
    pub dmarc: bool,
    pub mx: bool,
}

/// Receive bounce notification from chorus-mail.
pub async fn handle_bounce(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<BounceNotification>,
) -> Result<StatusCode, (StatusCode, String)> {
    // Validate shared secret
    let expected = state.config().bounce_secret.as_deref().unwrap_or("");
    let provided = headers
        .get("x-chorus-secret")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if expected.is_empty() || provided != expected {
        return Err((StatusCode::UNAUTHORIZED, "invalid secret".into()));
    }

    tracing::warn!(
        recipient = %body.recipient,
        reason = %body.reason,
        "bounce received from chorus-mail"
    );

    // TODO: look up message by provider message_id and update status to "bounced"
    // For now, log the bounce. Full implementation requires a message lookup by
    // provider_message_id which can be added as a follow-up.

    Ok(StatusCode::OK)
}

#[derive(Deserialize)]
pub struct DnsCheckQuery {
    pub domain: String,
}

/// Check DNS records for a domain.
pub async fn dns_check(
    axum::extract::Query(params): axum::extract::Query<DnsCheckQuery>,
) -> Result<Json<DnsCheckResult>, (StatusCode, String)> {
    let domain = &params.domain;

    let resolver = hickory_resolver::TokioResolver::builder_tokio()
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("resolver error: {}", e),
            )
        })?
        .build();

    let spf = check_txt_record(&resolver, domain, "v=spf1").await;
    let dkim = check_txt_record(
        &resolver,
        &format!("chorus._domainkey.{}", domain),
        "v=DKIM1",
    )
    .await;
    let dmarc = check_txt_record(&resolver, &format!("_dmarc.{}", domain), "v=DMARC1").await;
    let mx = check_mx_record(&resolver, domain).await;

    Ok(Json(DnsCheckResult {
        spf,
        dkim,
        dmarc,
        mx,
    }))
}

async fn check_txt_record(
    resolver: &hickory_resolver::TokioResolver,
    name: &str,
    prefix: &str,
) -> bool {
    match resolver.txt_lookup(name).await {
        Ok(lookup) => lookup.iter().any(|txt| txt.to_string().contains(prefix)),
        Err(_) => false,
    }
}

async fn check_mx_record(resolver: &hickory_resolver::TokioResolver, domain: &str) -> bool {
    resolver.mx_lookup(domain).await.is_ok()
}
