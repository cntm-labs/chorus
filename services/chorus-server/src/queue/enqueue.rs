use chrono::Utc;
use std::sync::Arc;

use crate::app::AppState;

/// Push a send job onto the Redis queue.
pub async fn job(state: &AppState, job: &super::SendJob) -> anyhow::Result<()> {
    let mut conn = state.redis.get_multiplexed_tokio_connection().await?;
    let payload = serde_json::to_string(job)?;
    redis::cmd("LPUSH")
        .arg(super::QUEUE_KEY)
        .arg(payload)
        .query_async::<i64>(&mut conn)
        .await?;
    Ok(())
}

/// Enqueue a job and dispatch the `message.queued` webhook event.
pub async fn notify(state: &Arc<AppState>, job: &super::SendJob) -> anyhow::Result<()> {
    self::job(state, job).await?;

    let payload = super::webhook_dispatch::WebhookPayload {
        event: "message.queued".into(),
        message_id: job.message_id,
        channel: job.channel.clone(),
        provider: None,
        status: "queued".into(),
        timestamp: Utc::now().to_rfc3339(),
    };
    super::webhook_dispatch::dispatch_webhooks(state, job.account_id, "message.queued", &payload)
        .await;

    Ok(())
}
