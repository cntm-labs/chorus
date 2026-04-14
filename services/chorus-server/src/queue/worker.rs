use chorus_core::types::{EmailMessage, SmsMessage};
use chrono::Utc;
use std::sync::Arc;

use crate::app::AppState;
use crate::config::Config;

/// Spawn N worker tasks that process jobs from the main queue.
pub fn spawn_workers(state: Arc<AppState>, config: Arc<Config>, concurrency: usize) {
    for i in 0..concurrency {
        let state = Arc::clone(&state);
        let config = Arc::clone(&config);
        tokio::spawn(async move {
            tracing::info!(worker = i, "queue worker started");
            loop {
                if let Err(e) = process_next_job(&state, &config).await {
                    tracing::error!(worker = i, "worker error: {e}");
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                }
            }
        });
    }
}

/// Block-pop the next job from Redis and process it.
async fn process_next_job(state: &Arc<AppState>, config: &Config) -> anyhow::Result<()> {
    let mut conn = state.redis.get_multiplexed_tokio_connection().await?;

    let result: Option<(String, String)> = redis::cmd("BRPOP")
        .arg(super::QUEUE_KEY)
        .arg(5)
        .query_async(&mut conn)
        .await?;

    let Some((_key, payload)) = result else {
        return Ok(());
    };

    let job: super::SendJob = serde_json::from_str(&payload)?;
    let repo = state.message_repo();

    // Load the message from DB
    let message = repo
        .find_by_id(job.message_id, job.account_id)
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?
        .ok_or_else(|| anyhow::anyhow!("message {} not found", job.message_id))?;

    // Max retries exceeded → DLQ
    if job.attempt >= super::MAX_RETRIES {
        repo.update_status(
            job.message_id,
            "failed",
            None,
            None,
            Some("max retries exceeded"),
        )
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;
        repo.insert_delivery_event(
            job.message_id,
            "failed",
            Some(serde_json::json!({"reason": "max retries exceeded", "attempts": job.attempt})),
        )
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;

        // Dispatch webhook for failure
        let webhook_payload = super::webhook_dispatch::WebhookPayload {
            event: "message.failed".into(),
            message_id: job.message_id,
            channel: job.channel.clone(),
            provider: None,
            status: "failed".into(),
            timestamp: Utc::now().to_rfc3339(),
        };
        super::webhook_dispatch::dispatch_webhooks(
            state,
            job.account_id,
            "message.failed",
            &webhook_payload,
        )
        .await;

        super::dead_letter::push_to_dlq(&state.redis, &job).await?;
        return Ok(());
    }

    // Build router based on environment
    let router = if job.environment == "test" {
        super::router_builder::build_test_router(&job.channel)
    } else {
        // Try per-account config, fall back to env defaults
        let configs = state
            .provider_config_repo()
            .list_by_account_channel(job.account_id, &job.channel)
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))?;

        if configs.is_empty() {
            super::router_builder::build_router_from_env(config, &job.channel)?
        } else {
            super::router_builder::build_router_from_configs(&configs)?
        }
    };

    // Send via chorus-core
    let send_result = match job.channel.as_str() {
        "sms" => {
            let msg = SmsMessage {
                to: message.recipient.clone(),
                body: message.body.clone(),
                from: message.sender.clone(),
            };
            router.send_sms(&msg).await
        }
        "email" => {
            let msg = EmailMessage {
                to: message.recipient.clone(),
                subject: message.subject.clone().unwrap_or_default(),
                html_body: message.body.clone(),
                text_body: message.body.clone(),
                from: message.sender.clone(),
            };
            router.send_email(&msg).await
        }
        _ => anyhow::bail!("unknown channel: {}", job.channel),
    };

    match send_result {
        Ok(result) => {
            // Dispatch message.sent (provider accepted)
            let sent_payload = super::webhook_dispatch::WebhookPayload {
                event: "message.sent".into(),
                message_id: job.message_id,
                channel: job.channel.clone(),
                provider: Some(result.provider.clone()),
                status: "sent".into(),
                timestamp: Utc::now().to_rfc3339(),
            };
            super::webhook_dispatch::dispatch_webhooks(
                state,
                job.account_id,
                "message.sent",
                &sent_payload,
            )
            .await;

            repo.update_status(
                job.message_id,
                "delivered",
                Some(&result.provider),
                Some(&result.message_id),
                None,
            )
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))?;
            repo.insert_delivery_event(
                job.message_id,
                "delivered",
                Some(serde_json::json!({
                    "provider": result.provider,
                    "provider_message_id": result.message_id,
                })),
            )
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))?;

            // Dispatch webhook
            let webhook_payload = super::webhook_dispatch::WebhookPayload {
                event: "message.delivered".into(),
                message_id: job.message_id,
                channel: job.channel.clone(),
                provider: Some(result.provider.clone()),
                status: "delivered".into(),
                timestamp: Utc::now().to_rfc3339(),
            };
            super::webhook_dispatch::dispatch_webhooks(
                state,
                job.account_id,
                "message.delivered",
                &webhook_payload,
            )
            .await;
        }
        Err(e) => {
            let error_msg = e.to_string();
            repo.update_status(job.message_id, "retrying", None, None, Some(&error_msg))
                .await
                .map_err(|e| anyhow::anyhow!("{e}"))?;
            repo.insert_delivery_event(
                job.message_id,
                "failed_attempt",
                Some(serde_json::json!({
                    "attempt": job.attempt,
                    "error": error_msg,
                })),
            )
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))?;

            // Schedule retry with incremented attempt
            let retry_job = super::SendJob {
                attempt: job.attempt + 1,
                ..job
            };
            super::delayed::schedule_retry(&state.redis, &retry_job).await?;
        }
    }

    Ok(())
}
