use chrono::Utc;
use hmac::{Hmac, Mac};
use serde::Serialize;
use sha2::Sha256;
use std::sync::Arc;
use uuid::Uuid;

use crate::app::AppState;

type HmacSha256 = Hmac<Sha256>;

/// Webhook event payload sent to callback URLs.
#[derive(Serialize)]
pub struct WebhookPayload {
    pub event: String,
    pub message_id: Uuid,
    pub channel: String,
    pub provider: Option<String>,
    pub status: String,
    pub timestamp: String,
}

/// Dispatch webhook events for a message status change.
pub async fn dispatch_webhooks(
    state: &Arc<AppState>,
    account_id: Uuid,
    event: &str,
    payload: &WebhookPayload,
) {
    let webhooks = match state
        .webhook_repo()
        .list_by_account_event(account_id, event)
        .await
    {
        Ok(hooks) => hooks,
        Err(e) => {
            tracing::error!("failed to load webhooks: {e}");
            return;
        }
    };

    let body = match serde_json::to_string(payload) {
        Ok(b) => b,
        Err(e) => {
            tracing::error!("failed to serialize webhook payload: {e}");
            return;
        }
    };

    let client = reqwest::Client::new();
    let timestamp = Utc::now().timestamp().to_string();

    for webhook in webhooks {
        let signature = compute_signature(&webhook.secret, &body);

        let result = client
            .post(&webhook.url)
            .header("Content-Type", "application/json")
            .header("X-Chorus-Signature", &signature)
            .header("X-Chorus-Event", event)
            .header("X-Chorus-Timestamp", &timestamp)
            .body(body.clone())
            .send()
            .await;

        match result {
            Ok(resp) if resp.status().is_success() => {
                tracing::debug!(webhook_id = %webhook.id, "webhook delivered");
            }
            Ok(resp) => {
                tracing::warn!(
                    webhook_id = %webhook.id,
                    status = %resp.status(),
                    "webhook delivery failed"
                );
            }
            Err(e) => {
                tracing::warn!(webhook_id = %webhook.id, "webhook HTTP error: {e}");
            }
        }
    }
}

/// Compute HMAC-SHA256 signature for webhook payload.
fn compute_signature(secret: &str, body: &str) -> String {
    let mut mac =
        HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC accepts any key length");
    mac.update(body.as_bytes());
    hex::encode(mac.finalize().into_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signature_is_deterministic() {
        let sig1 = compute_signature("secret", "payload");
        let sig2 = compute_signature("secret", "payload");
        assert_eq!(sig1, sig2);
        assert_eq!(sig1.len(), 64); // SHA-256 hex = 64 chars
    }

    #[test]
    fn different_secrets_produce_different_signatures() {
        let sig1 = compute_signature("secret-a", "payload");
        let sig2 = compute_signature("secret-b", "payload");
        assert_ne!(sig1, sig2);
    }
}
