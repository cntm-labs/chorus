//! Verification orchestration: constants, code generator, pricing helpers,
//! Valkey access, rate limiting, smart routing, cleanup loop.
//!
//! See `docs/superpowers/specs/2026-05-06-verification-api-design.md`.

use rand::Rng as _;

/// Code length in digits.
pub const CODE_LENGTH: usize = 6;
/// Valkey TTL for the code, in seconds (5 minutes).
pub const TTL_SECONDS: u64 = 300;
/// Max times `/check` may be called with a wrong code before lockout.
pub const MAX_CHECK_ATTEMPTS: i32 = 5;
/// Max times `/resend` may be called per verification.
pub const MAX_RESEND_ATTEMPTS: i32 = 3;
/// Sliding-window rate limit per recipient (1 hour window).
pub const RATE_LIMIT_PER_RCPT_HOUR: u32 = 5;
/// Sliding-window rate limit per account (1 minute window).
pub const RATE_LIMIT_PER_ACCT_MINUTE: u32 = 100;

/// Generate a cryptographically random `CODE_LENGTH`-digit code.
pub fn generate_code() -> String {
    let n: u32 = rand::thread_rng().gen_range(0..1_000_000);
    format!("{n:06}")
}

/// Pricing lookup. Returns cost in micro-USD for a single delivery.
pub fn cost_for(channel: &str, recipient: &str) -> i64 {
    match channel {
        "email" => 100,
        "sms" => sms_cost_for_country(extract_country(recipient)),
        _ => 0,
    }
}

fn sms_cost_for_country(cc: &str) -> i64 {
    match cc {
        "US" | "CA" => 5_000,
        "TH" => 6_000,
        _ => 8_000,
    }
}

/// Map a leading E.164 prefix to an ISO country code.
/// Returns `"??"` when the prefix is unknown — caller treats as fallback rate.
fn extract_country(e164: &str) -> &'static str {
    let digits = e164.trim_start_matches('+');
    // Longest match first (greedy).
    const PREFIXES: &[(&str, &str)] = &[
        ("1", "US"),   // also CA — single rate applies
        ("44", "UK"),
        ("49", "DE"),
        ("66", "TH"),
        ("65", "SG"),
        ("81", "JP"),
        ("82", "KR"),
        ("86", "CN"),
    ];
    let mut best: &str = "??";
    let mut best_len: usize = 0;
    for (prefix, cc) in PREFIXES {
        if digits.starts_with(prefix) && prefix.len() > best_len {
            best = cc;
            best_len = prefix.len();
        }
    }
    best
}

use sha2::{Digest, Sha256};
use uuid::Uuid;

/// Result of an attempted code check.
#[derive(Debug, PartialEq, Eq)]
pub enum CheckCodeOutcome {
    /// Code matched; the Valkey entry has been deleted.
    Match,
    /// Code did not match. The entry remains until TTL or lockout.
    Mismatch,
    /// No entry exists for this id (TTL expired or already consumed/canceled).
    Gone,
}

fn valkey_key(id: Uuid) -> String {
    format!("verify:{id}")
}

/// Hash the recipient for use in keys and logs (avoid plaintext PII).
pub fn hash_recipient(recipient: &str) -> String {
    hex::encode(Sha256::digest(recipient.as_bytes()))
}

/// Store the code with TTL. Overwrites any previous entry (e.g. on resend).
pub async fn store_code(
    redis: &redis::Client,
    id: Uuid,
    recipient: &str,
    code: &str,
) -> anyhow::Result<()> {
    let key = valkey_key(id);
    let value = serde_json::json!({
        "code": code,
        "recipient_hash": hash_recipient(recipient),
    });
    let mut conn = redis.get_multiplexed_tokio_connection().await?;
    redis::cmd("SET")
        .arg(&key)
        .arg(value.to_string())
        .arg("EX")
        .arg(TTL_SECONDS)
        .query_async::<String>(&mut conn)
        .await?;
    Ok(())
}

/// Compare the provided code against the stored one.
/// On `Match` the entry is deleted. On `Mismatch` the entry is left alone
/// (caller increments the authoritative DB counter).
pub async fn check_code(
    redis: &redis::Client,
    id: Uuid,
    code: &str,
) -> anyhow::Result<CheckCodeOutcome> {
    let key = valkey_key(id);
    let mut conn = redis.get_multiplexed_tokio_connection().await?;
    let raw: Option<String> = redis::cmd("GET").arg(&key).query_async(&mut conn).await?;
    let Some(raw) = raw else { return Ok(CheckCodeOutcome::Gone) };
    let data: serde_json::Value = serde_json::from_str(&raw)?;
    let stored = data["code"].as_str().unwrap_or("");
    if stored == code {
        redis::cmd("DEL").arg(&key).query_async::<i64>(&mut conn).await?;
        Ok(CheckCodeOutcome::Match)
    } else {
        Ok(CheckCodeOutcome::Mismatch)
    }
}

/// Delete the Valkey entry (used by cancel and lockout paths).
pub async fn invalidate_code(redis: &redis::Client, id: Uuid) -> anyhow::Result<()> {
    let key = valkey_key(id);
    let mut conn = redis.get_multiplexed_tokio_connection().await?;
    redis::cmd("DEL").arg(&key).query_async::<i64>(&mut conn).await?;
    Ok(())
}

/// Reasons routing rejects a verification request.
#[derive(Debug)]
pub enum RoutingError {
    NoRecipient,
    InvalidPhone,
    InvalidEmail,
    NoEligibleChannel,
    RateLimitedRecipient { retry_after_sec: u64 },
    RateLimitedAccount { retry_after_sec: u64 },
    Db(crate::db::DbError),
    Internal(anyhow::Error),
}

/// Atomic sliding-window check + increment over two ZSET keys.
/// Returns:
///   0           = allowed (and both windows incremented),
///   recipient   = "rcpt"  → retry-after = oldest_score_recipient + window_ms - now
///   account     = "acct"
const RATE_LIMIT_LUA: &str = r#"
local key_rcpt    = KEYS[1]
local key_acct    = KEYS[2]
local now_ms      = tonumber(ARGV[1])
local window_rcpt = tonumber(ARGV[2])
local limit_rcpt  = tonumber(ARGV[3])
local window_acct = tonumber(ARGV[4])
local limit_acct  = tonumber(ARGV[5])
local member      = ARGV[6]

redis.call('ZREMRANGEBYSCORE', key_rcpt, 0, now_ms - window_rcpt)
local count_rcpt = redis.call('ZCARD', key_rcpt)
if count_rcpt >= limit_rcpt then
    local oldest = redis.call('ZRANGE', key_rcpt, 0, 0, 'WITHSCORES')
    return {'rcpt', tonumber(oldest[2]) + window_rcpt - now_ms}
end

redis.call('ZREMRANGEBYSCORE', key_acct, 0, now_ms - window_acct)
local count_acct = redis.call('ZCARD', key_acct)
if count_acct >= limit_acct then
    local oldest = redis.call('ZRANGE', key_acct, 0, 0, 'WITHSCORES')
    return {'acct', tonumber(oldest[2]) + window_acct - now_ms}
end

redis.call('ZADD', key_rcpt, now_ms, member)
redis.call('ZADD', key_acct, now_ms, member)
redis.call('EXPIRE', key_rcpt, math.floor(window_rcpt / 1000))
redis.call('EXPIRE', key_acct, math.floor(window_acct / 1000))
return {'ok', 0}
"#;

/// Apply both rate-limit layers atomically.
pub async fn check_rate_limits(
    redis: &redis::Client,
    account_id: Uuid,
    recipient_hash: &str,
) -> Result<(), RoutingError> {
    use std::time::{SystemTime, UNIX_EPOCH};
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| RoutingError::Internal(anyhow::anyhow!("clock: {e}")))?
        .as_millis() as u64;

    let key_rcpt = format!("verify:rl:rcpt:{recipient_hash}");
    let key_acct = format!("verify:rl:acct:{account_id}");
    let member = format!("{now_ms}:{}", Uuid::new_v4());

    let window_rcpt_ms: u64 = 60 * 60 * 1000;
    let window_acct_ms: u64 = 60 * 1000;

    let mut conn = redis
        .get_multiplexed_tokio_connection()
        .await
        .map_err(|e| RoutingError::Internal(anyhow::anyhow!(e)))?;

    let result: (String, i64) = redis::Script::new(RATE_LIMIT_LUA)
        .key(&key_rcpt)
        .key(&key_acct)
        .arg(now_ms)
        .arg(window_rcpt_ms)
        .arg(RATE_LIMIT_PER_RCPT_HOUR)
        .arg(window_acct_ms)
        .arg(RATE_LIMIT_PER_ACCT_MINUTE)
        .arg(member)
        .invoke_async(&mut conn)
        .await
        .map_err(|e| RoutingError::Internal(anyhow::anyhow!(e)))?;

    match result.0.as_str() {
        "ok" => Ok(()),
        "rcpt" => Err(RoutingError::RateLimitedRecipient {
            retry_after_sec: (result.1.max(0) as u64).div_ceil(1000),
        }),
        "acct" => Err(RoutingError::RateLimitedAccount {
            retry_after_sec: (result.1.max(0) as u64).div_ceil(1000),
        }),
        other => Err(RoutingError::Internal(anyhow::anyhow!(
            "unknown rate-limit outcome: {other}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_code_is_six_digits() {
        for _ in 0..100 {
            let c = generate_code();
            assert_eq!(c.len(), 6, "code = {c:?}");
            assert!(c.chars().all(|c| c.is_ascii_digit()), "code = {c:?}");
        }
    }

    #[test]
    fn cost_for_email_flat() {
        assert_eq!(cost_for("email", "alice@example.com"), 100);
    }

    #[test]
    fn cost_for_sms_us() {
        assert_eq!(cost_for("sms", "+14155552671"), 5_000);
    }

    #[test]
    fn cost_for_sms_thailand() {
        assert_eq!(cost_for("sms", "+66812345678"), 6_000);
    }

    #[test]
    fn cost_for_sms_unknown_country_fallback() {
        assert_eq!(cost_for("sms", "+33123456789"), 8_000);
    }

    #[test]
    fn cost_for_unknown_channel_is_zero() {
        assert_eq!(cost_for("whatsapp", "+66..."), 0);
    }
}
