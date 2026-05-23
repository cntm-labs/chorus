//! AES-GCM-256 envelope encryption keyed from env `CHORUS_ENCRYPTION_KEY`.
//!
//! Blob layout: `nonce(12) || ciphertext || tag(16)`.
//! See `docs/superpowers/specs/2026-05-19-totp-design.md` §4.1.

use aes_gcm::{
    aead::{Aead, AeadCore, KeyInit, OsRng},
    Aes256Gcm, Key, Nonce,
};
use anyhow::{anyhow, Result};
use base64::{engine::general_purpose::STANDARD, Engine};

const NONCE_LEN: usize = 12;
const TAG_LEN: usize = 16;
const KEY_LEN: usize = 32;

/// AES-GCM-256 envelope encryption keyed from env `CHORUS_ENCRYPTION_KEY`.
pub struct Encryptor {
    cipher: Aes256Gcm,
}

impl Encryptor {
    /// Load from `CHORUS_ENCRYPTION_KEY` env var (base64 of 32 random bytes).
    /// Fail-fast at startup if missing or malformed.
    pub fn from_env() -> Result<Self> {
        let b64 = std::env::var("CHORUS_ENCRYPTION_KEY")
            .map_err(|_| anyhow!("CHORUS_ENCRYPTION_KEY env var not set"))?;
        let bytes = STANDARD
            .decode(b64.trim())
            .map_err(|e| anyhow!("CHORUS_ENCRYPTION_KEY is not valid base64: {e}"))?;
        if bytes.len() != KEY_LEN {
            return Err(anyhow!(
                "CHORUS_ENCRYPTION_KEY must decode to {} bytes, got {}",
                KEY_LEN,
                bytes.len()
            ));
        }
        let key = Key::<Aes256Gcm>::from_slice(&bytes);
        Ok(Self {
            cipher: Aes256Gcm::new(key),
        })
    }

    /// Encrypt plaintext. Returns `nonce(12) || ciphertext || tag(16)`.
    pub fn encrypt(&self, plaintext: &[u8]) -> Result<Vec<u8>> {
        let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
        let ct = self
            .cipher
            .encrypt(&nonce, plaintext)
            .map_err(|e| anyhow!("encryption failed: {e}"))?;
        let mut blob = Vec::with_capacity(NONCE_LEN + ct.len());
        blob.extend_from_slice(&nonce);
        blob.extend_from_slice(&ct);
        Ok(blob)
    }

    /// Decrypt a blob produced by `encrypt`. Authentication failure → Err.
    pub fn decrypt(&self, blob: &[u8]) -> Result<Vec<u8>> {
        if blob.len() < NONCE_LEN + TAG_LEN {
            return Err(anyhow!("blob too short ({} bytes)", blob.len()));
        }
        let (nonce_bytes, ct) = blob.split_at(NONCE_LEN);
        let nonce = Nonce::from_slice(nonce_bytes);
        self.cipher
            .decrypt(nonce, ct)
            .map_err(|e| anyhow!("decryption failed: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Encryptor {
        let key_bytes = [42u8; KEY_LEN];
        let key = Key::<Aes256Gcm>::from_slice(&key_bytes);
        Encryptor {
            cipher: Aes256Gcm::new(key),
        }
    }

    #[test]
    fn roundtrip_empty() {
        let enc = fixture();
        let blob = enc.encrypt(b"").unwrap();
        assert_eq!(enc.decrypt(&blob).unwrap(), b"");
    }

    #[test]
    fn roundtrip_one_byte() {
        let enc = fixture();
        let blob = enc.encrypt(b"x").unwrap();
        assert_eq!(enc.decrypt(&blob).unwrap(), b"x");
    }

    #[test]
    fn roundtrip_totp_secret_length() {
        let enc = fixture();
        let secret = [0xABu8; 20];
        let blob = enc.encrypt(&secret).unwrap();
        assert_eq!(blob.len(), NONCE_LEN + 20 + TAG_LEN);
        assert_eq!(enc.decrypt(&blob).unwrap(), &secret[..]);
    }

    #[test]
    fn decrypt_rejects_short_blob() {
        let enc = fixture();
        assert!(enc.decrypt(&[0u8; 27]).is_err());
    }

    #[test]
    fn decrypt_rejects_tampered_ciphertext() {
        let enc = fixture();
        let mut blob = enc.encrypt(b"hello world").unwrap();
        let last = blob.len() - 1;
        blob[last] ^= 0x01;
        assert!(enc.decrypt(&blob).is_err());
    }

    #[test]
    fn decrypt_rejects_wrong_key() {
        let enc_a = fixture();
        let key_b = [99u8; KEY_LEN];
        let enc_b = Encryptor {
            cipher: Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&key_b)),
        };
        let blob = enc_a.encrypt(b"secret").unwrap();
        assert!(enc_b.decrypt(&blob).is_err());
    }

    #[test]
    fn encrypt_produces_unique_nonces() {
        let enc = fixture();
        let mut blobs = std::collections::HashSet::new();
        for _ in 0..100 {
            let blob = enc.encrypt(b"same plaintext").unwrap();
            assert!(blobs.insert(blob), "duplicate blob — nonce not random");
        }
    }

    #[test]
    fn from_env_rejects_missing() {
        let prev = std::env::var("CHORUS_ENCRYPTION_KEY").ok();
        std::env::remove_var("CHORUS_ENCRYPTION_KEY");
        let result = Encryptor::from_env();
        if let Some(v) = prev {
            std::env::set_var("CHORUS_ENCRYPTION_KEY", v);
        }
        assert!(result.is_err());
    }

    #[test]
    fn from_env_rejects_short_key() {
        let prev = std::env::var("CHORUS_ENCRYPTION_KEY").ok();
        std::env::set_var("CHORUS_ENCRYPTION_KEY", STANDARD.encode([0u8; 16]));
        let result = Encryptor::from_env();
        if let Some(v) = prev {
            std::env::set_var("CHORUS_ENCRYPTION_KEY", v);
        } else {
            std::env::remove_var("CHORUS_ENCRYPTION_KEY");
        }
        assert!(result.is_err());
    }

    #[test]
    fn from_env_rejects_bad_base64() {
        let prev = std::env::var("CHORUS_ENCRYPTION_KEY").ok();
        std::env::set_var("CHORUS_ENCRYPTION_KEY", "not!valid@base64");
        let result = Encryptor::from_env();
        if let Some(v) = prev {
            std::env::set_var("CHORUS_ENCRYPTION_KEY", v);
        } else {
            std::env::remove_var("CHORUS_ENCRYPTION_KEY");
        }
        assert!(result.is_err());
    }
}
