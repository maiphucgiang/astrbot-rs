+++ b/crates/astrbot-provider/src/zerooneai.rs
@@ -0,0 +1,66 @@
+//! 01.AI (零一万物) provider — OpenAI-compatible API
+//!
+//! 01.AI provides Yi series models via an OpenAI-compatible API.
+//! Docs: https://platform.01.ai/
+
+use async_trait::async_trait;
+use crate::openai::OpenAiProvider;
+use astrbot_core::provider::{Provider, ChatMessage, ChatConfig, ChatResponse, ChatStreamChunk, ModelInfo};
+use astrbot_core::errors::Result;
+use futures_util::Stream;
+
+/// 01.AI provider wrapper
+pub struct ZeroOneAiProvider {
+    inner: OpenAiProvider,
+}
+
+impl ZeroOneAiProvider {
+    pub fn new(id: String, api_key: String, model: String) -> Self {
+        let base_url = "https://api.01.ai/v1".to_string();
+        Self {
+            inner: OpenAiProvider::new(id, api_key, base_url, model),
+        }
+    }
+}
+
+#[async_trait]
+impl Provider for ZeroOneAiProvider {
+    fn id(&self) -> &str { self.inner.id() }
+    fn name(&self) -> &str { self.inner.name() }
+
+    async fn models(&self) -> Result<Vec<String>> { self.inner.models().await }
+
+    async fn chat(&self, messages: Vec<ChatMessage>, config: ChatConfig) -> Result<ChatResponse> {
+        self.inner.chat(messages, config).await
+    }
+
+    async fn chat_stream(
+        &self, messages: Vec<ChatMessage>, config: ChatConfig,
+    ) -> Result<Box<dyn Stream<Item = Result<ChatStreamChunk>> + Send>> {
+        self.inner.chat_stream(messages, config).await
+    }
+
+    async fn embedding(&self, texts: Vec<String>, model: Option<String>) -> Result<Vec<Vec<f32>>> {
+        self.inner.embedding(texts, model).await
+    }
+
+    async fn model_info(&self, model: &str) -> Result<ModelInfo> { self.inner.model_info(model).await }
+
+    async fn health_check(&self) -> Result<bool> { self.inner.health_check().await }
+}
+
+#[cfg(test)]
+mod tests {
+    use super::*;
+
+    #[test]
+    fn test_01ai_provider_creation() {
+        let p = ZeroOneAiProvider::new(
+            "01ai-1".to_string(),
+            "sk-01ai-test".to_string(),
+            "yi-large".to_string(),
+        );
+        assert_eq!(p.id(), "01ai-1");
+        assert_eq!(p.name(), "01ai-1");
+    }
+}
