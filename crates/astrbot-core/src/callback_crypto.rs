//! Webhook callback encryption / signature verification
//!
//! Provides HMAC-SHA256 signature verification for platform webhook callbacks
//! (WeChat Official, DingTalk, Feishu, WeCom, etc.)

use hmac::{Hmac, Mac};
use sha2::Sha256;
use base64::Engine;

/// Verify HMAC-SHA256 signature
pub fn verify_signature(secret: &str, body: &str, signature: &str) -> bool {
    type HmacSha256 = Hmac<Sha256>;
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes())
        .expect("HMAC can take key of any size");
    mac.update(body.as_bytes());
    let result = mac.finalize();
    let expected = base64::engine::general_purpose::STANDARD.encode(result.into_bytes());
    expected == signature
}

/// Sign body with HMAC-SHA256
pub fn sign_body(secret: &str, body: &str) -> String {
    type HmacSha256 = Hmac<Sha256>;
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes())
        .expect("HMAC can take key of any size");
    mac.update(body.as_bytes());
    let result = mac.finalize();
    base64::engine::general_purpose::STANDARD.encode(result.into_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sign_and_verify() {
        let secret = "my-secret";
        let body = r#"{"msg":"hello"}"#;
        let sig = sign_body(secret, body);
        assert!(verify_signature(secret, body, &sig));
        assert!(!verify_signature(secret, body, "invalid"));
    }

    #[test]
    fn test_verify_bad_secret() {
        let body = r#"{"msg":"hello"}"#;
        let sig = sign_body("secret-a", body);
        assert!(!verify_signature("secret-b", body, &sig));
    }
}
