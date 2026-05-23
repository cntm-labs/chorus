//! TOTP orchestration: constants, code computation, QR generation, backup codes,
//! rate-limit wrapper.
//!
//! See `docs/superpowers/specs/2026-05-19-totp-design.md`.

use rand::RngCore;
use sha2::{Digest, Sha256};

// ---- Constants (RFC 6238 defaults, locked in MVP) ----

pub const TOTP_DIGITS: u32 = 6;
pub const TOTP_PERIOD_SECS: u64 = 30;
pub const TOTP_ALGORITHM: &str = "SHA1";
pub const TOTP_WINDOW: i64 = 1; // ±1 step tolerance
pub const SECRET_BYTES: usize = 20; // RFC 4226 recommended (160 bits)
pub const BACKUP_CODE_COUNT: usize = 10;
pub const LOW_BACKUP_THRESHOLD: i64 = 3;

pub const RATE_LIMIT_VERIFY_PER_USER_MIN: u32 = 5;
pub const RATE_LIMIT_ACTIVATE_PER_USER_MIN: u32 = 10;
pub const RATE_LIMIT_ENROLL_PER_USER_MIN: u32 = 10;
pub const RATE_LIMIT_PER_ACCT_MIN: u32 = 100;

// ---- Secret + code generation ----

/// Generate a new 20-byte (160-bit) TOTP secret using OsRng.
pub fn generate_secret() -> [u8; SECRET_BYTES] {
    let mut bytes = [0u8; SECRET_BYTES];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    bytes
}

/// Compute the TOTP code for `secret` at the given unix time (seconds).
/// Always SHA1, 6 digits, 30 s step.
pub fn compute_totp(secret: &[u8], time_seconds: u64) -> String {
    use totp_lite::{totp_custom, Sha1};
    totp_custom::<Sha1>(
        TOTP_PERIOD_SECS,
        TOTP_DIGITS,
        secret,
        time_seconds,
    )
}

/// Verify `code` against the secret at the current step ± `TOTP_WINDOW`.
pub fn verify_totp_with_window(secret: &[u8], now_seconds: u64, code: &str) -> bool {
    let step = now_seconds / TOTP_PERIOD_SECS;
    for offset in -TOTP_WINDOW..=TOTP_WINDOW {
        let candidate_step = step as i64 + offset;
        if candidate_step < 0 {
            continue;
        }
        let candidate_time = (candidate_step as u64) * TOTP_PERIOD_SECS;
        if compute_totp(secret, candidate_time) == code {
            return true;
        }
    }
    false
}

// ---- otpauth URI ----

/// Encode bytes as RFC 4648 base32 with NO padding (TOTP convention).
pub fn base32_no_pad(bytes: &[u8]) -> String {
    base32::encode(base32::Alphabet::Rfc4648 { padding: false }, bytes)
}

/// Build the `otpauth://totp/...` URI per RFC 6238.
/// `issuer` and `label` are URL-encoded; `secret_base32` is the raw base32 string.
pub fn build_otpauth_uri(issuer: &str, label: &str, secret_base32: &str) -> String {
    let issuer_enc = urlencoding::encode(issuer);
    let label_enc = urlencoding::encode(label);
    let secret_enc = urlencoding::encode(secret_base32);
    format!(
        "otpauth://totp/{issuer_enc}:{label_enc}?secret={secret_enc}&issuer={issuer_enc}&algorithm=SHA1&digits=6&period=30"
    )
}

/// SHA-256 hash for use as recipient-style identifier (e.g. user_id in rate-limit keys).
pub fn hash_user_id(user_id: &str) -> String {
    hex::encode(Sha256::digest(user_id.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_secret_is_20_bytes() {
        let s = generate_secret();
        assert_eq!(s.len(), SECRET_BYTES);
    }

    #[test]
    fn generate_secret_uses_full_entropy() {
        let mut seen = std::collections::HashSet::new();
        for _ in 0..100 {
            let s = generate_secret();
            assert!(seen.insert(s), "duplicate secret in 100 generations");
        }
    }

    /// RFC 6238 Appendix B test vector (SHA-1).
    /// Secret = ASCII "12345678901234567890".
    #[test]
    fn compute_totp_rfc6238_t1_59() {
        let secret = b"12345678901234567890";
        assert_eq!(compute_totp(secret, 59), "287082");
    }

    #[test]
    fn compute_totp_rfc6238_t2_1111111109() {
        let secret = b"12345678901234567890";
        assert_eq!(compute_totp(secret, 1_111_111_109), "081804");
    }

    #[test]
    fn compute_totp_rfc6238_t3_1234567890() {
        let secret = b"12345678901234567890";
        // RFC 6238 Appendix B gives 8-digit "89005924" for SHA1; 6-digit is 89005924 % 10^6 = 005924.
        assert_eq!(compute_totp(secret, 1_234_567_890), "005924");
    }

    #[test]
    fn verify_totp_with_window_accepts_current_step() {
        let secret = b"12345678901234567890";
        assert!(verify_totp_with_window(secret, 59, "287082"));
    }

    #[test]
    fn verify_totp_with_window_accepts_previous_step() {
        let secret = b"12345678901234567890";
        // At t=89 (step=2), the previous step (step=1, t∈[30,59]) produced "287082".
        assert!(verify_totp_with_window(secret, 89, "287082"));
    }

    #[test]
    fn verify_totp_with_window_accepts_next_step() {
        let secret = b"12345678901234567890";
        // At t=29 (step=0), the next step (step=1, t∈[30,59]) produces "287082".
        assert!(verify_totp_with_window(secret, 29, "287082"));
    }

    #[test]
    fn verify_totp_with_window_rejects_outside_window() {
        let secret = b"12345678901234567890";
        // At t=120 (step=4), the code from step=1 (t=59) is ±3 steps away.
        assert!(!verify_totp_with_window(secret, 120, "287082"));
    }

    #[test]
    fn verify_totp_with_window_rejects_wrong_code() {
        let secret = b"12345678901234567890";
        assert!(!verify_totp_with_window(secret, 59, "000000"));
    }

    #[test]
    fn base32_no_pad_known_vector() {
        // RFC 4648 §10: "foobar" → "MZXW6YTBOI" (no padding).
        assert_eq!(base32_no_pad(b"foobar"), "MZXW6YTBOI");
    }

    #[test]
    fn otpauth_uri_format() {
        let uri = build_otpauth_uri("Acme", "alice", "JBSWY3DPEHPK3PXP");
        assert!(uri.starts_with("otpauth://totp/Acme:alice?"));
        assert!(uri.contains("secret=JBSWY3DPEHPK3PXP"));
        assert!(uri.contains("issuer=Acme"));
        assert!(uri.contains("algorithm=SHA1"));
        assert!(uri.contains("digits=6"));
        assert!(uri.contains("period=30"));
    }

    #[test]
    fn otpauth_uri_escapes_special_chars() {
        let uri = build_otpauth_uri("Acme Inc", "alice@app.com", "JBSWY3DPEHPK3PXP");
        assert!(uri.contains("Acme%20Inc"));
        assert!(uri.contains("alice%40app.com"));
    }

    #[test]
    fn hash_user_id_is_64_hex_chars() {
        let h = hash_user_id("alice@app.com");
        assert_eq!(h.len(), 64);
        assert!(h.chars().all(|c| c.is_ascii_hexdigit()));
    }
}
