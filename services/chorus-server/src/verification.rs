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
