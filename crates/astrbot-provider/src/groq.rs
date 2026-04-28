+++ b/crates/astrbot-provider/src/groq.rs
@@ -0,0 +1,87 @@
+//! Groq provider — OpenAI-compatible API
+//!
+//! Groq offers ultra-fast LLM inference via an OpenAI-compatible API.
+//! Docs: https://console.groq.com/docs/openai
+
+use async_trait::async_trait;
+use crate::openai::OpenAiProvider;
+use astrbot_core::provider::{Provider, ChatMessage, ChatConfig, ChatResponse, ChatStreamChunk, ModelInfo};
+use astrbot_core::errors::Result;
+use futures_util::Stream;
+
+/// Groq provider wrapper
+pub struct GroqProvider {
+    inner: OpenAiProvider,
+}
+
+impl GroqProvider {
+    pub fn new(id: String, api_key: String, model: String) -> Self {
+        let base_url = "https://api.groq.com/openai/v1".to_string();
+        Self {
+            inner: OpenAiProvider::new(id, api_key, base_url, model),
+        }
+    }
+}
+
+#[async_trait]
+impl Provider for GroqProvider {
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
+    fn test_groq_provider_creation() {
+        let p = GroqProvider::new(
+            "groq-1".to_string(),
+            "gsk_test".to_string(),
+            "llama-3.1-70b-versatile".to_string(),
+        );
+        assert_eq!(p.id(), "groq-1");
+        assert_eq!(p.name(), "groq-1");
+    }
+
+    #[test]
+    fn test_groq_models() {
+        let p = GroqProvider::new(
+            "groq-1".to_string(),
+            "gsk_test".to_string(),
+            "llama-3.1-70b-versatile".to_string(),
+        );
+        let models = tokio::runtime::Runtime::new().unwrap().block_on(p.models());
+        assert!(models.is_ok());
+        assert_eq!(models.unwrap(), vec!["llama-3.1-70b-versatile"]);
+    }
+}
