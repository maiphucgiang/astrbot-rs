use anyhow::{bail, Result};
use hmac::{Hmac, Mac};
use sha2::Sha256;
use std::collections::HashSet;
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

type HmacSha256 = Hmac<Sha256>;

lazy_static::lazy_static! {
    static ref NONCE_CACHE: Mutex<HashSet<String>> = Mutex::new(HashSet::new());
}

const MAX_TIMESTAMP_DRIFT_SECS: i64 = 300;
const NONCE_TTL_SECS: u64 = 600;

pub struct WebhookSecurity;

impl WebhookSecurity {
    pub fn verify(
        secret: &[u8],
        payload: &[u8],
        signature: &str,
        timestamp_str: &str,
        nonce: &str,
    ) -> Result<()> {
        let timestamp: i64 = timestamp_str
            .parse()
            .map_err(|_| anyhow::anyhow!("Invalid timestamp format"))?;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or(Duration::from_secs(0))
            .as_secs() as i64;

        if (now - timestamp).abs() > MAX_TIMESTAMP_DRIFT_SECS {
            bail!(
                "Webhook timestamp too old or in future: diff={}s",
                now - timestamp
            );
        }

        {
            let mut cache = NONCE_CACHE.lock().unwrap();
            if cache.contains(nonce) {
                bail!("Replay attack detected: nonce already used");
            }
            cache.insert(nonce.to_string());
        }

        let mut mac = HmacSha256::new_from_slice(secret)
            .map_err(|_| anyhow::anyhow!("Invalid HMAC key length"))?;
        mac.update(payload);
        let expected = hex::encode(mac.finalize().into_bytes());

        if !constant_time_eq::constant_time_eq(expected.as_bytes(), signature.as_bytes()) {
            bail!("Webhook signature mismatch");
        }

        Ok(())
    }

    /// 派生 nonce（用于没有原生 nonce 的平台，如 Discord）
    pub fn derive_nonce(payload: &[u8], timestamp: &str) -> String {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(payload);
        hasher.update(timestamp.as_bytes());
        hex::encode(hasher.finalize())[..16].to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_webhook_verify_success() {
        let secret = b"test-secret";
        let payload = b"{}";
        let timestamp = std::time::SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            .to_string();
        let nonce = "unique-nonce-123";

        let mut mac = HmacSha256::new_from_slice(secret).unwrap();
        mac.update(payload);
        let signature = hex::encode(mac.finalize().into_bytes());

        assert!(WebhookSecurity::verify(secret, payload, &signature, &timestamp, nonce).is_ok());
    }

    #[test]
    fn test_replay_attack() {
        let secret = b"test-secret";
        let payload = b"{}";
        let timestamp = std::time::SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            .to_string();
        let nonce = "replay-nonce-456";

        let mut mac = HmacSha256::new_from_slice(secret).unwrap();
        mac.update(payload);
        let signature = hex::encode(mac.finalize().into_bytes());

        WebhookSecurity::verify(secret, payload, &signature, &timestamp, nonce).unwrap();
        let result = WebhookSecurity::verify(secret, payload, &signature, &timestamp, nonce);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Replay"));
    }

    #[test]
    fn test_timestamp_drift() {
        let secret = b"test-secret";
        let payload = b"{}";
        let old_timestamp = "1000000000";
        let nonce = "old-nonce";
        let signature = "aaa";

        let result = WebhookSecurity::verify(secret, payload, signature, old_timestamp, nonce);
        assert!(result.is_err());
    }
}
