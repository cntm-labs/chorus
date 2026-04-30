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
    // Validate shared secret.
    let expected = state.config().bounce_secret.as_deref().unwrap_or("");
    let provided = headers
        .get("x-chorus-secret")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if expected.is_empty() || provided != expected {
        return Err((StatusCode::UNAUTHORIZED, "invalid secret".into()));
    }

    let pmid = body.message_id.trim_matches(|c| c == '<' || c == '>');

    let message = state
        .message_repo()
        .find_by_provider_message_id(pmid)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let Some(message) = message else {
        tracing::warn!(
            message_id = %pmid,
            recipient = %body.recipient,
            "bounce arrived for unknown provider_message_id; ignoring"
        );
        return Ok(StatusCode::OK);
    };

    // Use the recipient Chorus originally accepted (canonical), not the bounce envelope's
    // recipient (postfix may have rewritten via aliasing).
    let normalized = match crate::suppression::normalize(&message.channel, &message.recipient) {
        Ok(n) => n,
        Err(_) => {
            tracing::warn!(
                channel = %message.channel,
                recipient = %message.recipient,
                "could not normalize stored recipient — skipping suppression write"
            );
            return Ok(StatusCode::OK);
        }
    };

    // Three sequential writes; chorus-mail's bounce-handler shell `exit 0`s on
    // curl failure so postfix won't retry. Order matters: write the most
    // user-critical state (message status + audit trail) first, suppression
    // last. Worst case if a later write fails: recipient could receive a
    // re-send (recoverable on the next bounce) — better than message stuck
    // in `queued` after suppression was already written.
    state
        .message_repo()
        .update_status(message.id, "bounced", None, None, Some(&body.reason))
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    state
        .message_repo()
        .insert_delivery_event(
            message.id,
            "bounced",
            Some(serde_json::json!({
                "reason": body.reason,
                "source": "chorus-mail",
            })),
        )
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    // Discard the returned row + inserted flag; bounce path doesn't surface them.
    let _ = state
        .suppression_repo()
        .add(&crate::db::NewSuppression {
            account_id: message.account_id,
            channel: message.channel.clone(),
            recipient: normalized,
            reason: "hard_bounce".into(),
            source: "chorus-mail".into(),
        })
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    tracing::info!(
        message_id = %message.id,
        account_id = %message.account_id,
        recipient = %body.recipient,
        "suppression added from chorus-mail bounce"
    );

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
