+++ b/crates/astrbot-provider/src/openrouter.rs
@@ -0,0 +1,75 @@
+//! OpenRouter provider — OpenAI-compatible API
+//!
+//! OpenRouter provides unified access to 100+ LLMs via an OpenAI-compatible API.
+//! Docs: https://openrouter.ai/docs
+
+use async_trait::async_trait;
+use crate::openai::OpenAiProvider;
+use astrbot_core::provider::{Provider, ChatMessage, ChatConfig, ChatResponse, ChatStreamChunk, ModelInfo};
+use astrbot_core::errors::Result;
+use futures_util::Stream;
+
+/// OpenRouter provider wrapper
+pub struct OpenRouterProvider {
+    inner: OpenAiProvider,
+}
+
+impl OpenRouterProvider {
+    pub fn new(id: String, api_key: String, model: String) -> Self {
+        let base_url = "https://openrouter.ai/api/v1".to_string();
+        Self {
+            inner: OpenAiProvider::new(id, api_key, base_url, model),
+        }
+    }
+}
+
+#[async_trait]
+impl Provider for OpenRouterProvider {
+    fn id(&self) -> &str {
+        self.inner.id()
+    }
+
+    fn name(&self) -> &str {
+        self.inner.name()
+    }
+
+    async fn models(&self) -> Result<Vec<String>> {
+        self.inner.models().await
+    }
+
+    async fn chat(&self, messages: Vec<ChatMessage>, config: ChatConfig) -> Result<ChatResponse> {
+        self.inner.chat(messages, config).await
+    }
+
+    async fn chat_stream(&self, messages: Vec<ChatMessage>, config: ChatConfig) -> Result<Box<dyn Stream<Item = Result<ChatStreamChunk>> + Send>> {
+        self.inner.chat_stream(messages, config).await
+    }
+
+    async fn embedding(&self, texts: Vec<String>, model: Option<String>) -> Result<Vec<Vec<f32>>> {
+        self.inner.embedding(texts, model).await
+    }
+
+    async fn model_info(&self, model: &str) -> Result<ModelInfo> {
+        self.inner.model_info(model).await
+    }
+
+    async fn health_check(&self) -> Result<bool> {
+        self.inner.health_check().await
+    }
+}
+
+#[cfg(test)]
+mod tests {
+    use super::*;
+
+    #[test]
+    fn test_openrouter_provider_creation() {
+        let p = OpenRouterProvider::new(
+            "or-1".to_string(),
+            "sk-or-test".to_string(),
+            "anthropic/claude-3.5-sonnet".to_string(),
+        );
+        assert_eq!(p.id(), "or-1");
+        assert_eq!(p.name(), "or-1");
+    }
+}
