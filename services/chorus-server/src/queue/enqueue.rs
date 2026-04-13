use crate::app::AppState;

/// Push a send job onto the Redis queue.
pub async fn enqueue_job(state: &AppState, job: &super::SendJob) -> anyhow::Result<()> {
    let mut conn = state.redis.get_multiplexed_tokio_connection().await?;
    let payload = serde_json::to_string(job)?;
    redis::cmd("LPUSH")
        .arg(super::QUEUE_KEY)
        .arg(payload)
        .query_async::<i64>(&mut conn)
        .await?;
    Ok(())
}
