use sha2::{Digest, Sha256};
use uuid::Uuid;

/// OTP time-to-live in seconds (5 minutes).
const OTP_TTL_SECS: u64 = 300;
/// Maximum verification attempts before lockout.
const MAX_ATTEMPTS: i64 = 3;

/// Generate a cryptographically random 6-digit OTP code.
pub fn generate_code() -> String {
    let n: u32 = rand::random::<u32>() % 1_000_000;
    format!("{n:06}")
}

/// Store an OTP in Redis with TTL. Returns the OTP ID for verification.
pub async fn store(redis: &redis::Client, recipient: &str, code: &str) -> anyhow::Result<Uuid> {
    let otp_id = Uuid::new_v4();
    let recipient_hash = hex::encode(Sha256::digest(recipient.as_bytes()));

    let key = format!("otp:{otp_id}");
    let value = serde_json::json!({
        "code": code,
        "recipient_hash": recipient_hash,
        "attempts": 0,
    });

    let mut conn = redis.get_multiplexed_tokio_connection().await?;
    redis::cmd("SET")
        .arg(&key)
        .arg(value.to_string())
        .arg("EX")
        .arg(OTP_TTL_SECS)
        .query_async::<String>(&mut conn)
        .await?;

    Ok(otp_id)
}

/// Verify an OTP code. Returns `Ok(true)` on match, `Ok(false)` on mismatch.
/// Deletes the OTP on success. Returns error if expired or max attempts reached.
pub async fn verify(redis: &redis::Client, otp_id: Uuid, code: &str) -> anyhow::Result<bool> {
    let key = format!("otp:{otp_id}");
    let mut conn = redis.get_multiplexed_tokio_connection().await?;

    let raw: Option<String> = redis::cmd("GET").arg(&key).query_async(&mut conn).await?;

    let Some(raw) = raw else {
        anyhow::bail!("OTP expired or not found");
    };

    let mut data: serde_json::Value = serde_json::from_str(&raw)?;

    let attempts = data["attempts"].as_i64().unwrap_or(0);
    if attempts >= MAX_ATTEMPTS {
        // Delete on lockout
        redis::cmd("DEL")
            .arg(&key)
            .query_async::<i64>(&mut conn)
            .await?;
        anyhow::bail!("too many attempts");
    }

    let stored_code = data["code"].as_str().unwrap_or("");
    if stored_code == code {
        // Success — delete OTP
        redis::cmd("DEL")
            .arg(&key)
            .query_async::<i64>(&mut conn)
            .await?;
        return Ok(true);
    }

    // Increment attempts
    data["attempts"] = serde_json::json!(attempts + 1);
    let ttl: i64 = redis::cmd("TTL").arg(&key).query_async(&mut conn).await?;
    if ttl > 0 {
        redis::cmd("SET")
            .arg(&key)
            .arg(data.to_string())
            .arg("EX")
            .arg(ttl)
            .query_async::<String>(&mut conn)
            .await?;
    }

    Ok(false)
}
