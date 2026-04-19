use chrono::Utc;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::Instrument;
use uuid::Uuid;

use crate::app::AppState;

type HmacSha256 = Hmac<Sha256>;

/// Maximum webhook delivery attempts before giving up.
const MAX_WEBHOOK_ATTEMPTS: i32 = 3;

/// Redis key for delayed webhook retry queue.
const WEBHOOK_DELAYED_KEY: &str = "chorus:webhook_delayed";

/// Webhook event payload sent to callback URLs.
#[derive(Serialize, Deserialize, Clone)]
pub struct WebhookPayload {
    pub event: String,
    pub message_id: Uuid,
    pub channel: String,
    pub provider: Option<String>,
    pub status: String,
    pub timestamp: String,
}

/// A webhook delivery job for retry queue.
#[derive(Serialize, Deserialize)]
struct WebhookJob {
    webhook_id: Uuid,
    url: String,
    secret: String,
    event: String,
    body: String,
    attempt: i32,
}

/// Dispatch webhook events for a message status change.
pub async fn dispatch_webhooks(
    state: &Arc<AppState>,
    account_id: Uuid,
    event: &str,
    payload: &WebhookPayload,
) {
    let span = tracing::info_span!(
        "dispatch_webhooks",
        account_id = %account_id,
        event = %event,
        message_id = %payload.message_id,
    );

    async {
        let webhooks = match state
            .webhook_repo()
            .list_by_account_event(account_id, event)
            .await
        {
            Ok(hooks) => hooks,
            Err(e) => {
                tracing::error!(error = %e, "failed to load webhooks");
                return;
            }
        };

        let body = match serde_json::to_string(payload) {
            Ok(b) => b,
            Err(e) => {
                tracing::error!(error = %e, "failed to serialize webhook payload");
                return;
            }
        };

        for webhook in webhooks {
            let delivered = deliver_webhook(
                state.http_client(),
                &webhook.url,
                &webhook.secret,
                event,
                &body,
            )
            .await;

            if !delivered {
                let job = WebhookJob {
                    webhook_id: webhook.id,
                    url: webhook.url,
                    secret: webhook.secret,
                    event: event.to_string(),
                    body: body.clone(),
                    attempt: 1,
                };
                if let Err(e) = schedule_webhook_retry(&state.redis, &job).await {
                    tracing::error!(
                        webhook_id = %job.webhook_id,
                        error = %e,
                        "failed to schedule webhook retry"
                    );
                }
            }
        }
    }
    .instrument(span)
    .await;
}

/// Attempt to deliver a webhook payload to a URL. Returns true on success.
async fn deliver_webhook(
    client: &reqwest::Client,
    url: &str,
    secret: &str,
    event: &str,
    body: &str,
) -> bool {
    let span = tracing::info_span!("deliver_webhook", url = %url, event = %event);

    async {
        let signature = compute_signature(secret, body);
        let timestamp = Utc::now().timestamp().to_string();

        let result = client
            .post(url)
            .header("Content-Type", "application/json")
            .header("X-Chorus-Signature", &signature)
            .header("X-Chorus-Event", event)
            .header("X-Chorus-Timestamp", &timestamp)
            .body(body.to_string())
            .send()
            .await;

        match result {
            Ok(resp) if resp.status().is_success() => {
                tracing::debug!("webhook delivered");
                metrics::counter!("chorus_webhook_deliveries_total", "event" => event.to_string(), "status" => "success").increment(1);
                true
            }
            Ok(resp) => {
                tracing::warn!(status = %resp.status(), "webhook delivery failed");
                metrics::counter!("chorus_webhook_deliveries_total", "event" => event.to_string(), "status" => "failed").increment(1);
                false
            }
            Err(e) => {
                tracing::warn!(error = %e, "webhook HTTP error");
                metrics::counter!("chorus_webhook_deliveries_total", "event" => event.to_string(), "status" => "error").increment(1);
                false
            }
        }
    }
    .instrument(span)
    .await
}

/// Schedule a webhook job for retry with exponential backoff.
async fn schedule_webhook_retry(redis: &redis::Client, job: &WebhookJob) -> anyhow::Result<()> {
    let mut conn = redis.get_multiplexed_tokio_connection().await?;
    let backoff_secs = 1u64 << job.attempt.min(10) as u64; // 2, 4, 8, ...
    let retry_at = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() + backoff_secs;

    let payload = serde_json::to_string(job)?;
    redis::cmd("ZADD")
        .arg(WEBHOOK_DELAYED_KEY)
        .arg(retry_at)
        .arg(payload)
        .query_async::<i64>(&mut conn)
        .await?;

    tracing::debug!(
        webhook_id = %job.webhook_id,
        attempt = job.attempt,
        backoff_secs,
        "scheduled webhook retry"
    );

    Ok(())
}

/// Spawn the webhook retry poller that processes due webhook retries.
pub fn spawn_webhook_retry_poller(redis: redis::Client, http_client: reqwest::Client) {
    tokio::spawn(async move {
        tracing::info!("webhook retry poller started");
        loop {
            if let Err(e) = poll_webhook_retries(&redis, &http_client).await {
                tracing::error!(error = %e, "webhook retry poller error");
            }
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        }
    });
}

/// Poll and process due webhook retry jobs.
async fn poll_webhook_retries(
    redis: &redis::Client,
    http_client: &reqwest::Client,
) -> anyhow::Result<()> {
    let mut conn = redis.get_multiplexed_tokio_connection().await?;
    let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();

    let jobs: Vec<String> = redis::cmd("ZRANGEBYSCORE")
        .arg(WEBHOOK_DELAYED_KEY)
        .arg("-inf")
        .arg(now)
        .query_async(&mut conn)
        .await?;

    for payload in jobs {
        let removed: i64 = redis::cmd("ZREM")
            .arg(WEBHOOK_DELAYED_KEY)
            .arg(&payload)
            .query_async(&mut conn)
            .await?;

        if removed == 0 {
            continue; // Another poller got it
        }

        let job: WebhookJob = serde_json::from_str(&payload)?;
        let delivered =
            deliver_webhook(http_client, &job.url, &job.secret, &job.event, &job.body).await;

        if !delivered && job.attempt < MAX_WEBHOOK_ATTEMPTS {
            let retry_job = WebhookJob {
                attempt: job.attempt + 1,
                ..job
            };
            schedule_webhook_retry(redis, &retry_job).await?;
        } else if !delivered {
            tracing::warn!(
                webhook_id = %job.webhook_id,
                attempts = job.attempt,
                "webhook delivery permanently failed after max retries"
            );
        }
    }

    Ok(())
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

    #[test]
    fn max_webhook_attempts_is_reasonable() {
        assert_eq!(MAX_WEBHOOK_ATTEMPTS, 3);
    }
}
