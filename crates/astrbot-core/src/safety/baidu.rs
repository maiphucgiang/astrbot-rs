use crate::errors::{AstrBotError, Result};
use crate::message::MessageChain;
use crate::safety::{SafetyResult, SafetyStrategy};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{error, info, warn};

/// Baidu AI Content Moderation (百度内容审核)
///
/// Docs: https://ai.baidu.com/ai-doc/ANTIPORN/Vk3e6cgja
///
/// Two-step flow:
/// 1. OAuth: client_id + client_secret → access_token
/// 2. Censor: text + access_token → conclusion (合规/不合规)
pub struct BaiduContentSafety {
    name: String,
    client_id: String,
    client_secret: String,
    /// Cached access_token + expiry
    token_state: Arc<Mutex<TokenState>>,
    client: reqwest::Client,
}

#[derive(Debug, Clone)]
struct TokenState {
    access_token: String,
    expires_at: chrono::DateTime<chrono::Utc>,
}

/// Baidu OAuth response
#[derive(Debug, Deserialize)]
struct BaiduTokenResponse {
    access_token: String,
    expires_in: i64, // seconds
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    error_description: Option<String>,
}

/// Baidu text censor request
#[derive(Debug, Serialize)]
struct BaiduCensorRequest {
    text: String,
}

/// Baidu text censor response
#[derive(Debug, Deserialize)]
struct BaiduCensorResponse {
    /// 合规结论: 1=合规, 2=不合规, 3=疑似, 4=审核失败
    conclusion: Option<String>,
    /// 详细审核结果
    #[serde(default)]
    data: Vec<BaiduCensorDataItem>,
    /// 错误码
    #[serde(default)]
    error_code: Option<i64>,
    /// 错误信息
    #[serde(default)]
    error_msg: Option<String>,
}

#[derive(Debug, Deserialize)]
struct BaiduCensorDataItem {
    /// 子结论类型
    #[serde(default, rename = "type")]
    item_type: Option<String>,
    /// 命中关键词
    #[serde(default)]
    hit: Vec<String>,
    /// 审核建议: pass/review/reject
    #[serde(default)]
    suggestion: Option<String>,
    /// 子类型说明
    #[serde(default)]
    msg: Option<String>,
}

impl BaiduContentSafety {
    pub fn new(
        name: impl Into<String>,
        client_id: impl Into<String>,
        client_secret: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            client_id: client_id.into(),
            client_secret: client_secret.into(),
            token_state: Arc::new(Mutex::new(TokenState {
                access_token: String::new(),
                expires_at: chrono::DateTime::UNIX_EPOCH,
            })),
            client: reqwest::Client::new(),
        }
    }

    /// Create with a custom HTTP client.
    pub fn with_client(
        name: impl Into<String>,
        client_id: impl Into<String>,
        client_secret: impl Into<String>,
        client: reqwest::Client,
    ) -> Self {
        Self {
            name: name.into(),
            client_id: client_id.into(),
            client_secret: client_secret.into(),
            token_state: Arc::new(Mutex::new(TokenState {
                access_token: String::new(),
                expires_at: chrono::DateTime::UNIX_EPOCH,
            })),
            client,
        }
    }

    /// Ensure we have a valid access_token, refreshing if needed.
    async fn ensure_token(&self) -> Result<String> {
        let mut state = self.token_state.lock().await;
        let now = chrono::Utc::now();
        // Refresh if token expires within 5 minutes
        if state.expires_at > now + chrono::Duration::minutes(5) && !state.access_token.is_empty() {
            return Ok(state.access_token.clone());
        }

        let url = format!(
            "https://aip.baidubce.com/oauth/2.0/token?grant_type=client_credentials&client_id={}&client_secret={}",
            &self.client_id, &self.client_secret
        );

        let response = self
            .client
            .post(&url)
            .send()
            .await
            .map_err(|e| AstrBotError::Network(format!("Baidu OAuth request failed: {}", e)))?;

        let status = response.status();
        if !status.is_success() {
            let text = response.text().await.unwrap_or_default();
            error!("[BaiduContentSafety] OAuth HTTP {} — {}", status, text);
            return Err(AstrBotError::Network(format!(
                "Baidu OAuth returned HTTP {}: {}",
                status, text
            )));
        }

        let payload: BaiduTokenResponse = response.json().await.map_err(|e| {
            AstrBotError::Serialization(format!("Baidu OAuth JSON parse error: {}", e))
        })?;

        if let Some(err) = payload.error {
            return Err(AstrBotError::Internal(format!(
                "Baidu OAuth error: {} — {}",
                err,
                payload.error_description.unwrap_or_default()
            )));
        }

        state.access_token = payload.access_token.clone();
        state.expires_at = now + chrono::Duration::seconds(payload.expires_in);
        info!(
            "[BaiduContentSafety] token refreshed, expires in {}s",
            payload.expires_in
        );

        Ok(payload.access_token)
    }

    /// Call Baidu text censor API.
    async fn censor_text(&self, text: &str) -> Result<SafetyResult> {
        if text.is_empty() {
            return Ok(SafetyResult::Safe);
        }

        let token = self.ensure_token().await?;
        let url =
            "https://aip.baidubce.com/rest/2.0/solution/v1/text_censor/v2/user_defined".to_string();

        let body = BaiduCensorRequest {
            text: text.to_string(),
        };

        let response = self
            .client
            .post(&url)
            .query(&[("access_token", &token)])
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| AstrBotError::Network(format!("Baidu censor request failed: {}", e)))?;

        let status = response.status();
        if !status.is_success() {
            let text = response.text().await.unwrap_or_default();
            error!("[BaiduContentSafety] Censor HTTP {} — {}", status, text);
            return Err(AstrBotError::Network(format!(
                "Baidu censor returned HTTP {}: {}",
                status, text
            )));
        }

        let payload: BaiduCensorResponse = response.json().await.map_err(|e| {
            AstrBotError::Serialization(format!("Baidu censor JSON parse error: {}", e))
        })?;

        // error_code handling
        if let Some(code) = payload.error_code {
            if code != 0 && code != 110000 {
                return Ok(SafetyResult::Error {
                    message: format!(
                        "Baidu censor API error: {} (code {})",
                        payload.error_msg.unwrap_or_default(),
                        code
                    ),
                });
            }
        }

        // conclusion mapping
        match payload.conclusion.as_deref() {
            Some("合规") | Some("1") => Ok(SafetyResult::Safe),
            Some("不合规") | Some("2") => {
                let reasons: Vec<String> = payload
                    .data
                    .iter()
                    .filter_map(|item| {
                        let msg = item.msg.clone().unwrap_or_default();
                        let hit = item.hit.join(", ");
                        if !msg.is_empty() && !hit.is_empty() {
                            Some(format!("{} (命中: {})", msg, hit))
                        } else if !msg.is_empty() {
                            Some(msg)
                        } else {
                            item.suggestion.clone()
                        }
                    })
                    .collect();
                let reason = if reasons.is_empty() {
                    "Content violates Baidu moderation policy".to_string()
                } else {
                    reasons.join("; ")
                };
                Ok(SafetyResult::Violation {
                    reason,
                    strategy: self.name.clone(),
                })
            }
            Some("疑似") | Some("3") => Ok(SafetyResult::Violation {
                reason: "Content flagged as suspicious by Baidu moderation".to_string(),
                strategy: self.name.clone(),
            }),
            Some("审核失败") | Some("4") => Ok(SafetyResult::Error {
                message: "Baidu moderation audit failed".to_string(),
            }),
            other => {
                warn!("[BaiduContentSafety] unknown conclusion: {:?}", other);
                Ok(SafetyResult::Safe)
            }
        }
    }
}

#[async_trait]
impl SafetyStrategy for BaiduContentSafety {
    fn name(&self) -> &str {
        &self.name
    }

    async fn check(&self, chain: &MessageChain) -> SafetyResult {
        let text = chain.plain_text();
        match self.censor_text(&text).await {
            Ok(result) => result,
            Err(e) => {
                error!("[BaiduContentSafety] check failed: {}", e);
                SafetyResult::Error {
                    message: format!("Baidu moderation check failed: {}", e),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::MessageChain;

    #[test]
    fn test_baidu_safety_creation() {
        let safety = BaiduContentSafety::new("baidu", "test-client-id", "test-client-secret");
        assert_eq!(safety.name(), "baidu");
    }

    #[tokio::test]
    async fn test_baidu_empty_text_safe() {
        let safety = BaiduContentSafety::new("baidu", "test-client-id", "test-client-secret");
        let chain = MessageChain::new();
        let result = safety.check(&chain).await;
        assert_eq!(result, SafetyResult::Safe);
    }
}
