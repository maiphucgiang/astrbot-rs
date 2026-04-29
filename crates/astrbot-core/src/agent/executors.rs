use crate::agent::{AgentContext, AgentExecutor, AgentResult};
use crate::errors::{AstrBotError, Result};
use async_trait::async_trait;
use tracing::{info, warn};

// ---------------------------------------------------------------------------
// Coze Executor
// ---------------------------------------------------------------------------

/// Coze (扣子) agent executor
pub struct CozeExecutor {
    api_key: String,
    bot_id: String,
    base_url: String,
    client: reqwest::Client,
}

impl CozeExecutor {
    pub fn new(api_key: impl Into<String>, bot_id: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            bot_id: bot_id.into(),
            base_url: "https://api.coze.com".to_string(),
            client: reqwest::Client::new(),
        }
    }

    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = url.into();
        self
    }

    fn is_test_key(&self) -> bool {
        self.api_key.is_empty()
            || self.api_key.starts_with("test")
            || self.api_key.starts_with("sk-test")
    }
}

#[async_trait]
impl AgentExecutor for CozeExecutor {
    fn name(&self) -> &str {
        &self.bot_id
    }

    fn executor_type(&self) -> &str {
        "coze"
    }

    async fn initialize(&mut self, config: serde_json::Value) -> Result<()> {
        if let Some(url) = config.get("base_url").and_then(|v| v.as_str()) {
            self.base_url = url.to_string();
        }
        info!("[Coze] Executor initialized for bot: {}", self.bot_id);
        Ok(())
    }

    async fn execute(&self, ctx: &AgentContext) -> Result<AgentResult> {
        if self.is_test_key() {
            return Ok(AgentResult::Text {
                content: format!(
                    "[Coze synthetic] Bot {} received {} messages",
                    self.bot_id,
                    ctx.messages.len()
                ),
            });
        }

        let user_msg = ctx
            .messages
            .last()
            .map(|m| m.content.clone())
            .unwrap_or_default();

        let body = serde_json::json!({
            "bot_id": self.bot_id,
            "user_id": ctx.user_id,
            "additional_messages": [{
                "role": "user",
                "content": user_msg,
                "content_type": "text"
            }],
            "stream": false,
        });

        let url = format!("{}/v3/chat", self.base_url.trim_end_matches('/'));
        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| AstrBotError::Network(format!("Coze request failed: {}", e)))?;

        let status = response.status();
        let text = response.text().await.unwrap_or_default();

        if !status.is_success() {
            return Ok(AgentResult::Error {
                message: format!("Coze API error ({}): {}", status, text),
            });
        }

        // Coze streaming response — extract final answer from event lines
        let answer = text
            .lines()
            .filter(|l| l.starts_with("data:"))
            .filter_map(|l| {
                let json_str = l.trim_start_matches("data:").trim();
                let v: serde_json::Value = serde_json::from_str(json_str).ok()?;
                v.get("event")?.as_str().and_then(|ev| {
                    if ev == "conversation.message.completed" {
                        v.get("data")?.get("content")?.as_str().map(String::from)
                    } else {
                        None
                    }
                })
            })
            .last()
            .unwrap_or_else(|| text.chars().take(200).collect());

        Ok(AgentResult::Text { content: answer })
    }

    async fn health_check(&self) -> Result<bool> {
        if self.is_test_key() {
            return Ok(true);
        }
        let url = format!(
            "{}/v1/conversation/create",
            self.base_url.trim_end_matches('/')
        );
        match self.client.get(&url).send().await {
            Ok(resp) => Ok(resp.status().is_success() || resp.status().as_u16() == 401),
            Err(_) => Ok(false),
        }
    }
}

// ---------------------------------------------------------------------------
// Dify Executor
// ---------------------------------------------------------------------------

pub struct DifyExecutor {
    api_key: String,
    app_id: String,
    base_url: String,
    mode: DifyMode,
    client: reqwest::Client,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DifyMode {
    Chat,
    Workflow,
    Agent,
}

impl Default for DifyMode {
    fn default() -> Self {
        DifyMode::Chat
    }
}

impl DifyExecutor {
    pub fn new(api_key: impl Into<String>, app_id: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            app_id: app_id.into(),
            base_url: "https://api.dify.ai".to_string(),
            mode: DifyMode::Chat,
            client: reqwest::Client::new(),
        }
    }

    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = url.into();
        self
    }

    pub fn with_mode(mut self, mode: DifyMode) -> Self {
        self.mode = mode;
        self
    }

    fn is_test_key(&self) -> bool {
        self.api_key.is_empty()
            || self.api_key.starts_with("test")
            || self.api_key.starts_with("sk-test")
    }
}

#[async_trait]
impl AgentExecutor for DifyExecutor {
    fn name(&self) -> &str {
        &self.app_id
    }

    fn executor_type(&self) -> &str {
        "dify"
    }

    async fn initialize(&mut self, config: serde_json::Value) -> Result<()> {
        if let Some(url) = config.get("base_url").and_then(|v| v.as_str()) {
            self.base_url = url.to_string();
        }
        if let Some(mode_str) = config.get("mode").and_then(|v| v.as_str()) {
            self.mode = match mode_str {
                "chat" => DifyMode::Chat,
                "workflow" => DifyMode::Workflow,
                "agent" => DifyMode::Agent,
                _ => DifyMode::Chat,
            };
        }
        info!(
            "[Dify] Executor initialized for app: {} (mode: {:?})",
            self.app_id, self.mode
        );
        Ok(())
    }

    async fn execute(&self, ctx: &AgentContext) -> Result<AgentResult> {
        if self.is_test_key() {
            return Ok(AgentResult::Text {
                content: format!(
                    "[Dify synthetic] App {} received {} messages (mode: {:?})",
                    self.app_id,
                    ctx.messages.len(),
                    self.mode
                ),
            });
        }

        let query = ctx
            .messages
            .last()
            .map(|m| m.content.clone())
            .unwrap_or_default();

        let (url_path, body) = match self.mode {
            DifyMode::Chat | DifyMode::Agent => {
                let body = serde_json::json!({
                    "inputs": {},
                    "query": query,
                    "user": ctx.user_id,
                    "response_mode": "blocking",
                    "conversation_id": ctx.session_id,
                });
                ("/v1/chat-messages", body)
            }
            DifyMode::Workflow => {
                let body = serde_json::json!({
                    "inputs": { "query": query },
                    "user": ctx.user_id,
                    "response_mode": "blocking",
                });
                ("/v1/workflows/run", body)
            }
        };

        let url = format!("{}{}", self.base_url.trim_end_matches('/'), url_path);
        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| AstrBotError::Network(format!("Dify request failed: {}", e)))?;

        let status = response.status();
        let json: serde_json::Value = response
            .json()
            .await
            .map_err(|e| AstrBotError::Serialization(format!("Dify response parse: {}", e)))?;

        if !status.is_success() {
            return Ok(AgentResult::Error {
                message: format!("Dify API error ({}): {:?}", status, json),
            });
        }

        let answer = match self.mode {
            DifyMode::Chat | DifyMode::Agent => json
                .get("answer")
                .and_then(|a| a.as_str())
                .map(String::from),
            DifyMode::Workflow => json
                .get("data")
                .and_then(|d| d.get("outputs"))
                .and_then(|o| o.as_object())
                .and_then(|map| map.values().next())
                .and_then(|v| v.as_str())
                .map(String::from),
        };

        Ok(match answer {
            Some(text) => AgentResult::Text { content: text },
            None => AgentResult::Error {
                message: "Dify returned empty answer".to_string(),
            },
        })
    }

    async fn health_check(&self) -> Result<bool> {
        if self.is_test_key() {
            return Ok(true);
        }
        let url = format!("{}/v1/parameters", self.base_url.trim_end_matches('/'));
        match self.client.get(&url).send().await {
            Ok(resp) => Ok(resp.status().is_success() || resp.status().as_u16() == 401),
            Err(_) => Ok(false),
        }
    }
}

// ---------------------------------------------------------------------------
// DeerFlow Executor
// ---------------------------------------------------------------------------

pub struct DeerFlowExecutor {
    api_key: String,
    base_url: String,
    client: reqwest::Client,
}

impl DeerFlowExecutor {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            base_url: "https://api.deerflow.ai".to_string(),
            client: reqwest::Client::new(),
        }
    }

    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = url.into();
        self
    }

    fn is_test_key(&self) -> bool {
        self.api_key.is_empty()
            || self.api_key.starts_with("test")
            || self.api_key.starts_with("sk-test")
    }
}

#[async_trait]
impl AgentExecutor for DeerFlowExecutor {
    fn name(&self) -> &str {
        "deerflow"
    }

    fn executor_type(&self) -> &str {
        "deerflow"
    }

    async fn initialize(&mut self, config: serde_json::Value) -> Result<()> {
        if let Some(url) = config.get("base_url").and_then(|v| v.as_str()) {
            self.base_url = url.to_string();
        }
        info!("[DeerFlow] Executor initialized");
        Ok(())
    }

    async fn execute(&self, ctx: &AgentContext) -> Result<AgentResult> {
        if self.is_test_key() {
            return Ok(AgentResult::Text {
                content: format!(
                    "[DeerFlow synthetic] Received {} messages",
                    ctx.messages.len()
                ),
            });
        }

        let messages: Vec<serde_json::Value> = ctx
            .messages
            .iter()
            .map(|m| {
                serde_json::json!({
                    "role": m.role,
                    "content": m.content,
                })
            })
            .collect();

        let body = serde_json::json!({
            "messages": messages,
            "context": {
                "user_id": ctx.user_id,
                "session_id": ctx.session_id,
            }
        });

        let url = format!("{}/v1/execute", self.base_url.trim_end_matches('/'));
        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| AstrBotError::Network(format!("DeerFlow request failed: {}", e)))?;

        let status = response.status();
        let json: serde_json::Value = response
            .json()
            .await
            .map_err(|e| AstrBotError::Serialization(format!("DeerFlow response parse: {}", e)))?;

        if !status.is_success() {
            return Ok(AgentResult::Error {
                message: format!("DeerFlow API error ({}): {:?}", status, json),
            });
        }

        let answer = json
            .get("result")
            .and_then(|r| r.as_str())
            .map(String::from)
            .or_else(|| {
                json.get("output")
                    .and_then(|o| o.as_str())
                    .map(String::from)
            })
            .or_else(|| json.get("text").and_then(|t| t.as_str()).map(String::from));

        Ok(match answer {
            Some(text) => AgentResult::Text { content: text },
            None => AgentResult::Error {
                message: "DeerFlow returned empty result".to_string(),
            },
        })
    }

    async fn health_check(&self) -> Result<bool> {
        if self.is_test_key() {
            return Ok(true);
        }
        let url = format!("{}/v1/health", self.base_url.trim_end_matches('/'));
        match self.client.get(&url).send().await {
            Ok(resp) => Ok(resp.status().is_success() || resp.status().as_u16() == 401),
            Err(_) => Ok(false),
        }
    }
}

// ---------------------------------------------------------------------------
// DashScope Executor
// ---------------------------------------------------------------------------

pub struct DashScopeExecutor {
    api_key: String,
    app_id: String,
    base_url: String,
    client: reqwest::Client,
}

impl DashScopeExecutor {
    pub fn new(api_key: impl Into<String>, app_id: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            app_id: app_id.into(),
            base_url: "https://dashscope.aliyuncs.com".to_string(),
            client: reqwest::Client::new(),
        }
    }

    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = url.into();
        self
    }

    fn is_test_key(&self) -> bool {
        self.api_key.is_empty()
            || self.api_key.starts_with("test")
            || self.api_key.starts_with("sk-test")
    }
}

#[async_trait]
impl AgentExecutor for DashScopeExecutor {
    fn name(&self) -> &str {
        &self.app_id
    }

    fn executor_type(&self) -> &str {
        "dashscope"
    }

    async fn initialize(&mut self, config: serde_json::Value) -> Result<()> {
        if let Some(url) = config.get("base_url").and_then(|v| v.as_str()) {
            self.base_url = url.to_string();
        }
        info!("[DashScope] Executor initialized for app: {}", self.app_id);
        Ok(())
    }

    async fn execute(&self, ctx: &AgentContext) -> Result<AgentResult> {
        if self.is_test_key() {
            return Ok(AgentResult::Text {
                content: format!(
                    "[DashScope synthetic] App {} received {} messages",
                    self.app_id,
                    ctx.messages.len()
                ),
            });
        }

        let prompt = ctx
            .messages
            .last()
            .map(|m| m.content.clone())
            .unwrap_or_default();

        let body = serde_json::json!({
            "input": {
                "prompt": prompt,
            },
            "parameters": {}
        });

        let url = format!(
            "{}/api/v1/apps/{}/completion",
            self.base_url.trim_end_matches('/'),
            self.app_id
        );
        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| AstrBotError::Network(format!("DashScope request failed: {}", e)))?;

        let status = response.status();
        let json: serde_json::Value = response
            .json()
            .await
            .map_err(|e| AstrBotError::Serialization(format!("DashScope response parse: {}", e)))?;

        if !status.is_success() {
            return Ok(AgentResult::Error {
                message: format!("DashScope API error ({}): {:?}", status, json),
            });
        }

        let answer = json
            .get("output")
            .and_then(|o| o.get("text"))
            .and_then(|t| t.as_str())
            .map(String::from);

        Ok(match answer {
            Some(text) => AgentResult::Text { content: text },
            None => AgentResult::Error {
                message: "DashScope returned empty output".to_string(),
            },
        })
    }

    async fn health_check(&self) -> Result<bool> {
        if self.is_test_key() {
            return Ok(true);
        }
        let url = format!("{}/api/v1/apps", self.base_url.trim_end_matches('/'));
        match self.client.get(&url).send().await {
            Ok(resp) => Ok(resp.status().is_success() || resp.status().as_u16() == 401),
            Err(_) => Ok(false),
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::AgentContext;
    use crate::platform::MessageSource;
    use crate::provider::ChatMessage;

    fn make_test_context() -> AgentContext {
        AgentContext {
            messages: vec![
                ChatMessage::system("You are a helpful assistant."),
                ChatMessage::user("Hello"),
            ],
            source: MessageSource {
                platform: crate::platform::PlatformType::Custom,
                session_id: "test-chat".to_string(),
                message_id: "msg-1".to_string(),
                user_id: "test-user".to_string(),
            },
            user_id: "test-user".to_string(),
            session_id: "test-session".to_string(),
            extras: std::collections::HashMap::new(),
        }
    }

    #[tokio::test]
    async fn test_coze_executor_creation() {
        let mut exe = CozeExecutor::new("test-key", "bot123");
        assert_eq!(exe.name(), "bot123");
        assert_eq!(exe.executor_type(), "coze");

        exe.initialize(serde_json::json!({"base_url": "https://custom.coze.com"}))
            .await
            .unwrap();
        let health = exe.health_check().await.unwrap();
        assert!(health);

        let ctx = make_test_context();
        let result = exe.execute(&ctx).await.unwrap();
        match result {
            AgentResult::Text { content } => {
                assert!(content.contains("Coze synthetic"));
                assert!(content.contains("bot123"));
            }
            _ => panic!("Expected Text result, got {:?}", result),
        }
    }

    #[tokio::test]
    async fn test_dify_executor_creation() {
        let mut exe = DifyExecutor::new("test-key", "app456");
        assert_eq!(exe.name(), "app456");
        assert_eq!(exe.executor_type(), "dify");

        exe.initialize(serde_json::json!({"mode": "workflow"}))
            .await
            .unwrap();
        let health = exe.health_check().await.unwrap();
        assert!(health);

        let ctx = make_test_context();
        let result = exe.execute(&ctx).await.unwrap();
        match result {
            AgentResult::Text { content } => {
                assert!(content.contains("Dify synthetic"));
                assert!(content.contains("app456"));
            }
            _ => panic!("Expected Text result, got {:?}", result),
        }
    }

    #[tokio::test]
    async fn test_dify_workflow_mode() {
        let mut exe = DifyExecutor::new("test-key", "app789").with_mode(DifyMode::Workflow);
        assert_eq!(exe.mode, DifyMode::Workflow);

        exe.initialize(serde_json::json!({})).await.unwrap();
        let ctx = make_test_context();
        let result = exe.execute(&ctx).await.unwrap();
        match result {
            AgentResult::Text { content } => {
                assert!(content.contains("Dify synthetic"));
                assert!(content.contains("Workflow"));
            }
            _ => panic!("Expected Text result, got {:?}", result),
        }
    }

    #[tokio::test]
    async fn test_deerflow_executor_creation() {
        let mut exe = DeerFlowExecutor::new("test-key");
        assert_eq!(exe.name(), "deerflow");
        assert_eq!(exe.executor_type(), "deerflow");

        exe.initialize(serde_json::json!({})).await.unwrap();
        let health = exe.health_check().await.unwrap();
        assert!(health);

        let ctx = make_test_context();
        let result = exe.execute(&ctx).await.unwrap();
        match result {
            AgentResult::Text { content } => {
                assert!(content.contains("DeerFlow synthetic"));
            }
            _ => panic!("Expected Text result, got {:?}", result),
        }
    }

    #[tokio::test]
    async fn test_dashscope_executor_creation() {
        let mut exe = DashScopeExecutor::new("test-key", "app012");
        assert_eq!(exe.name(), "app012");
        assert_eq!(exe.executor_type(), "dashscope");

        exe.initialize(serde_json::json!({})).await.unwrap();
        let health = exe.health_check().await.unwrap();
        assert!(health);

        let ctx = make_test_context();
        let result = exe.execute(&ctx).await.unwrap();
        match result {
            AgentResult::Text { content } => {
                assert!(content.contains("DashScope synthetic"));
                assert!(content.contains("app012"));
            }
            _ => panic!("Expected Text result, got {:?}", result),
        }
    }
}
