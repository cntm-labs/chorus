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
