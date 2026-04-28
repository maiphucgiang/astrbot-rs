use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Serialize, Deserialize)]
pub struct Claims {
    pub sub: String,
    pub exp: usize,
    pub iat: usize,
    pub jti: String,
    pub fingerprint: String,
}

pub fn current_timestamp() -> usize {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as usize
}

/// 生成 JWT token（15min access + 7d refresh 分离）
pub fn generate_access_token(
    user_id: &str,
    jti: &str,
    fingerprint: &str,
    secret: &[u8],
) -> Result<String, jsonwebtoken::errors::Error> {
    let now = current_timestamp();
    let claims = Claims {
        sub: user_id.to_string(),
        iat: now,
        exp: now + 900, // 15 minutes
        jti: jti.to_string(),
        fingerprint: fingerprint.to_string(),
    };

    let header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::HS256);
    let key = jsonwebtoken::EncodingKey::from_secret(secret);
    jsonwebtoken::encode(&header, &claims, &key)
}

/// 校验 token（含 jti 撤销检查 — 调用方需自行查 revoked_tokens 表）
pub fn verify_token(
    token: &str,
    secret: &[u8],
) -> Result<jsonwebtoken::TokenData<Claims>, jsonwebtoken::errors::Error> {
    let validation = jsonwebtoken::Validation::new(jsonwebtoken::Algorithm::HS256);
    let key = jsonwebtoken::DecodingKey::from_secret(secret);
    jsonwebtoken::decode::<Claims>(token, &key, &validation)
}

/// 生成唯一 jti
pub fn generate_jti() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// 安全响应头
pub fn security_headers() -> Vec<(&'static str, &'static str)> {
    vec![
        (
            "Content-Security-Policy",
            "default-src 'self'; script-src 'self'; object-src 'none'; base-uri 'self';",
        ),
        ("X-Content-Type-Options", "nosniff"),
        ("X-Frame-Options", "DENY"),
        ("Referrer-Policy", "strict-origin-when-cross-origin"),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_token_roundtrip() {
        let secret = b"test-secret-at-least-32-bytes-long!";
        let jti = generate_jti();
        let token = generate_access_token("user123", &jti, "fp1", secret).unwrap();
        let decoded = verify_token(&token, secret).unwrap();
        assert_eq!(decoded.claims.sub, "user123");
        assert_eq!(decoded.claims.jti, jti);
    }

    #[test]
    fn test_expired_token_fails() {
        let secret = b"test-secret-at-least-32-bytes-long!";
        let claims = Claims {
            sub: "user".to_string(),
            iat: 1000,
            exp: 1001,
            jti: "jti".to_string(),
            fingerprint: "fp".to_string(),
        };
        let header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::HS256);
        let key = jsonwebtoken::EncodingKey::from_secret(secret);
        let token = jsonwebtoken::encode(&header, &claims, &key).unwrap();

        let result = verify_token(&token, secret);
        assert!(result.is_err());
    }
}
