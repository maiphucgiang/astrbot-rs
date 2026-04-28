//! Feishu authentication: tenant_access_token, app_access_token, caching

use dashmap::DashMap;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use sha2::Digest;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing::{debug, error, info};

use crate::{ApiResponse, AppCredentials, FeishuError, Result};

const TOKEN_URL: &str = "https://open.feishu.cn/open-apis/auth/v3/tenant_access_token/internal";
const BASE_URL: &str = "https://open.feishu.cn/open-apis";

/// Cached token entry
struct TokenEntry {
    token: String,
    expires_at: Instant,
}

/// Feishu authentication manager
#[derive(Clone)]
pub struct FeishuAuth {
    client: Client,
    creds: AppCredentials,
    token_cache: Arc<DashMap<String, TokenEntry>>,
    base_url: String,
}

impl FeishuAuth {
    pub fn new(creds: AppCredentials) -> Self {
        Self {
            client: Client::new(),
            creds,
            token_cache: Arc::new(DashMap::new()),
            base_url: BASE_URL.to_string(),
        }
    }

    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = url.into();
        self
    }

    /// Get (or refresh) tenant_access_token
    pub async fn tenant_access_token(&self) -> Result<String> {
        let cache_key = format!("{}:tenant", self.creds.app_id);

        // Check cache
        if let Some(entry) = self.token_cache.get(&cache_key) {
            if entry.expires_at > Instant::now() {
                debug!("Using cached tenant_access_token");
                return Ok(entry.token.clone());
            }
        }

        // Fetch new token
        let body = TokenRequestBody {
            app_id: self.creds.app_id.clone(),
            app_secret: self.creds.app_secret.clone(),
        };

        let resp = self
            .client
            .post(format!("{}/auth/v3/tenant_access_token/internal", self.base_url))
            .json(&body)
            .send()
            .await
            .map_err(FeishuError::Http)?;

        let _status = resp.status();
        let api_resp: ApiResponse<TokenResponseData> = resp
            .json()
            .await
            .map_err(FeishuError::Http)?;

        if api_resp.code != 0 || api_resp.data.is_none() {
            error!("Auth failed: {} - {}", api_resp.code, api_resp.msg);
            return Err(FeishuError::Auth(format!(
                "{} (code: {})",
                api_resp.msg, api_resp.code
            )));
        }

        let data = api_resp.data.unwrap();
        let _expires_in = Duration::from_secs(data.expire as u64);
        // Refresh 5 minutes early
        let safe_expires = Duration::from_secs((data.expire as u64).saturating_sub(300));

        let entry = TokenEntry {
            token: data.tenant_access_token.clone(),
            expires_at: Instant::now() + safe_expires,
        };

        self.token_cache.insert(cache_key, entry);
        info!("Tenant token refreshed, expires in {}s", data.expire);

        Ok(data.tenant_access_token)
    }

    /// Build an authenticated request builder
    pub async fn auth_request(
        &self,
        method: reqwest::Method,
        path: impl AsRef<str>,
    ) -> Result<reqwest::RequestBuilder> {
        let token = self.tenant_access_token().await?;
        let url = format!("{}{}", self.base_url, path.as_ref());
        let req = self
            .client
            .request(method, &url)
            .header("Authorization", format!("Bearer {}", token))
            .header("Content-Type", "application/json");
        Ok(req)
    }

    /// Verify webhook signature
    pub fn verify_webhook(
        &self,
        timestamp: &str,
        nonce: &str,
        body: &str,
        signature: &str,
    ) -> Result<bool> {
        let encrypt_key = self
            .creds
            .encrypt_key
            .as_ref()
            .ok_or_else(|| FeishuError::Config("Missing encrypt_key for webhook".into()))?;

        let to_sign = format!("{}{}{}{}", timestamp, nonce, encrypt_key, body);
        let hash = sha2::Sha256::digest(to_sign.as_bytes());
        let expected = hex::encode(hash);
        Ok(expected == signature)
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    pub fn client(&self) -> &Client {
        &self.client
    }
}

#[derive(Serialize)]
struct TokenRequestBody {
    app_id: String,
    app_secret: String,
}

#[derive(Default, Deserialize)]
struct TokenResponseData {
    tenant_access_token: String,
    expire: i32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_auth_new() {
        let creds = AppCredentials {
            app_id: "cli_xxx".into(),
            app_secret: "sec_xxx".into(),
            encrypt_key: Some("key".into()),
            verification_token: None,
        };
        let auth = FeishuAuth::new(creds);
        assert_eq!(auth.base_url(), BASE_URL);
    }

    #[test]
    fn test_verify_webhook_missing_key() {
        let creds = AppCredentials {
            app_id: "cli_xxx".into(),
            app_secret: "sec_xxx".into(),
            encrypt_key: None,
            verification_token: None,
        };
        let auth = FeishuAuth::new(creds);
        let result = auth.verify_webhook("1", "2", "body", "sig");
        assert!(result.is_err());
    }
}
