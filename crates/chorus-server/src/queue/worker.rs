use std::sync::Arc;

use crate::app::AppState;

const QUEUE_KEY: &str = "chorus:jobs";
const MAX_RETRIES: i32 = 3;

/// Spawn the background worker that processes queued messages.
pub fn spawn_worker(state: Arc<AppState>) {
    tokio::spawn(async move {
        tracing::info!("queue worker started");
        loop {
            if let Err(e) = process_next_job(&state).await {
                tracing::error!("worker error: {e}");
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            }
        }
    });
}

/// Block-pop the next job from Redis and process it.
async fn process_next_job(state: &Arc<AppState>) -> anyhow::Result<()> {
    let mut conn = state.redis.get_multiplexed_tokio_connection().await?;

    // BRPOP blocks until a job is available (timeout 5s)
    let result: Option<(String, String)> =
        redis::cmd("BRPOP")
            .arg(QUEUE_KEY)
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

    if message.attempts >= MAX_RETRIES {
        repo.update_status(job.message_id, "failed", None, None, Some("max retries exceeded"))
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        repo.insert_delivery_event(
            job.message_id,
            "failed",
            Some(serde_json::json!({"reason": "max retries exceeded"})),
        )
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;
        return Ok(());
    }

    // In test environment, just mark as delivered without sending
    if job.environment == "test" {
        repo.update_status(job.message_id, "delivered", Some("mock"), Some("test-mock-id"), None)
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        repo.insert_delivery_event(
            job.message_id,
            "delivered",
            Some(serde_json::json!({"provider": "mock", "environment": "test"})),
        )
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;
        return Ok(());
    }

    // Live sending will be implemented when provider config is wired up
    repo.update_status(job.message_id, "sent", None, None, None)
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    repo.insert_delivery_event(job.message_id, "sent", None)
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    Ok(())
}
