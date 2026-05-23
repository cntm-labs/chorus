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

// ---- Backup codes ----

/// Generate `BACKUP_CODE_COUNT` plaintext codes of format `[a-z0-9]{4}-[a-z0-9]{5}`.
pub fn generate_backup_codes() -> Vec<String> {
    use rand::Rng;
    let mut rng = rand::rngs::OsRng;
    const CHARSET: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";
    let mut codes = Vec::with_capacity(BACKUP_CODE_COUNT);
    for _ in 0..BACKUP_CODE_COUNT {
        let mut s = String::with_capacity(10);
        for i in 0..9 {
            if i == 4 {
                s.push('-');
            }
            let idx = rng.gen_range(0..CHARSET.len());
            s.push(CHARSET[idx] as char);
        }
        codes.push(s);
    }
    codes
}

/// SHA-256 of plaintext backup code.
pub fn hash_backup_code(code: &str) -> Vec<u8> {
    Sha256::digest(code.as_bytes()).to_vec()
}

/// True if `s` looks like a backup code (`xxxx-xxxxx`), so the verify path
/// can skip TOTP computation and consult the backup-codes table.
pub fn is_backup_code_format(s: &str) -> bool {
    s.len() == 10
        && s.chars().nth(4) == Some('-')
        && s.chars()
            .enumerate()
            .all(|(i, c)| i == 4 || (c.is_ascii_lowercase() || c.is_ascii_digit()))
}

// ---- QR code generation ----

/// Render the otpauth URI as a 256×256 PNG and return as `data:image/png;base64,...`.
pub fn qr_png_data_uri(otpauth_uri: &str) -> anyhow::Result<String> {
    use base64::Engine;
    use fast_qr::convert::image::ImageBuilder;
    use fast_qr::convert::Builder;
    use fast_qr::convert::Shape;
    use fast_qr::qr::QRBuilder;

    let qr = QRBuilder::new(otpauth_uri)
        .build()
        .map_err(|e| anyhow::anyhow!("QR build failed: {e:?}"))?;
    let png = ImageBuilder::default()
        .shape(Shape::Square)
        .fit_width(256)
        .to_pixmap(&qr)
        .encode_png()
        .map_err(|e| anyhow::anyhow!("PNG encode failed: {e}"))?;
    let b64 = base64::engine::general_purpose::STANDARD.encode(&png);
    Ok(format!("data:image/png;base64,{b64}"))
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

    #[test]
    fn generate_backup_codes_returns_10() {
        let codes = generate_backup_codes();
        assert_eq!(codes.len(), BACKUP_CODE_COUNT);
    }

    #[test]
    fn generate_backup_codes_match_format() {
        let codes = generate_backup_codes();
        for c in codes {
            assert!(is_backup_code_format(&c), "bad format: {c:?}");
        }
    }

    #[test]
    fn generate_backup_codes_are_unique() {
        let mut seen = std::collections::HashSet::new();
        for _ in 0..100 {
            for c in generate_backup_codes() {
                assert!(seen.insert(c.clone()), "duplicate code: {c}");
            }
        }
    }

    #[test]
    fn is_backup_code_format_true_positive() {
        assert!(is_backup_code_format("a3f8-9d2cx"));
        assert!(is_backup_code_format("0000-00000"));
    }

    #[test]
    fn is_backup_code_format_rejects_totp() {
        assert!(!is_backup_code_format("483921"));
    }

    #[test]
    fn is_backup_code_format_rejects_wrong_length() {
        assert!(!is_backup_code_format("a3f8-9d2c"));   // 9 chars
        assert!(!is_backup_code_format("a3f8-9d2cxx")); // 11 chars
    }

    #[test]
    fn is_backup_code_format_rejects_uppercase() {
        assert!(!is_backup_code_format("A3F8-9D2CX"));
    }

    #[test]
    fn is_backup_code_format_rejects_missing_hyphen() {
        assert!(!is_backup_code_format("a3f8x9d2cx"));
    }

    #[test]
    fn hash_backup_code_is_32_bytes() {
        assert_eq!(hash_backup_code("a3f8-9d2cx").len(), 32);
    }

    #[test]
    fn hash_backup_code_is_deterministic() {
        assert_eq!(hash_backup_code("a3f8-9d2cx"), hash_backup_code("a3f8-9d2cx"));
    }

    #[test]
    fn qr_png_data_uri_returns_data_uri_prefix() {
        let uri = "otpauth://totp/Acme:alice?secret=JBSWY3DPEHPK3PXP";
        let result = qr_png_data_uri(uri).unwrap();
        assert!(result.starts_with("data:image/png;base64,"));
    }

    #[test]
    fn qr_png_data_uri_is_valid_png() {
        use base64::Engine;
        let uri = "otpauth://totp/Acme:alice?secret=JBSWY3DPEHPK3PXP";
        let data_uri = qr_png_data_uri(uri).unwrap();
        let b64 = data_uri.strip_prefix("data:image/png;base64,").unwrap();
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(b64)
            .unwrap();
        // PNG magic: 89 50 4E 47 0D 0A 1A 0A
        assert_eq!(&bytes[..8], &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]);
    }
}
