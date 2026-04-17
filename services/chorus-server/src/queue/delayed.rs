use std::time::{Instant, SystemTime, UNIX_EPOCH};

/// Spawn the delayed queue poller that moves due jobs back to the main queue.
pub fn spawn_delayed_poller(redis: redis::Client) {
    tokio::spawn(async move {
        tracing::info!("delayed queue poller started");
        loop {
            if let Err(e) = poll_delayed_jobs(&redis).await {
                tracing::error!("delayed poller error: {e}");
            }
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
    });
}

/// Move jobs whose retry-at timestamp has passed back to the main queue.
async fn poll_delayed_jobs(redis: &redis::Client) -> anyhow::Result<()> {
    let mut conn = redis.get_multiplexed_tokio_connection().await?;
    let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();

    // Fetch jobs that are due (score <= now)
    let start = Instant::now();
    let jobs: Vec<String> = redis::cmd("ZRANGEBYSCORE")
        .arg(super::DELAYED_KEY)
        .arg("-inf")
        .arg(now)
        .query_async(&mut conn)
        .await?;
    super::record_redis_duration!("zrangebyscore", start);

    for payload in jobs {
        // Atomically remove from delayed set — if ZREM returns 0, another poller got it
        let removed: i64 = redis::cmd("ZREM")
            .arg(super::DELAYED_KEY)
            .arg(&payload)
            .query_async(&mut conn)
            .await?;

        if removed > 0 {
            redis::cmd("LPUSH")
                .arg(super::QUEUE_KEY)
                .arg(&payload)
                .query_async::<i64>(&mut conn)
                .await?;
            tracing::debug!("moved delayed job back to main queue");
        }
    }

    Ok(())
}

/// Schedule a job for retry after exponential backoff.
pub async fn schedule_retry(redis: &redis::Client, job: &super::SendJob) -> anyhow::Result<()> {
    let mut conn = redis.get_multiplexed_tokio_connection().await?;
    let backoff_secs = 1u64 << job.attempt.min(10) as u64; // 2^attempt: 1, 2, 4, ...
    let retry_at = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() + backoff_secs;

    let payload = serde_json::to_string(job)?;
    let start = Instant::now();
    redis::cmd("ZADD")
        .arg(super::DELAYED_KEY)
        .arg(retry_at)
        .arg(payload)
        .query_async::<i64>(&mut conn)
        .await?;
    super::record_redis_duration!("zadd", start);

    tracing::debug!(
        message_id = %job.message_id,
        attempt = job.attempt,
        backoff_secs,
        "scheduled retry"
    );

    Ok(())
}
