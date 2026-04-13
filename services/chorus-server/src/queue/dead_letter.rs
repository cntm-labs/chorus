/// Push a failed job to the dead letter queue.
pub async fn push_to_dlq(redis: &redis::Client, job: &super::SendJob) -> anyhow::Result<()> {
    let mut conn = redis.get_multiplexed_tokio_connection().await?;
    let payload = serde_json::to_string(job)?;

    redis::cmd("LPUSH")
        .arg(super::DEAD_LETTER_KEY)
        .arg(payload)
        .query_async::<i64>(&mut conn)
        .await?;

    tracing::warn!(message_id = %job.message_id, "job moved to dead letter queue");
    Ok(())
}
