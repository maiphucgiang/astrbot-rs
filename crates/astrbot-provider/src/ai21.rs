+++ b/crates/astrbot-provider/src/ai21.rs
@@ -0,0 +1,66 @@
+//! AI21 Labs provider — OpenAI-compatible API
+//!
+//! AI21 offers the Jamba and Jurassic model families via an OpenAI-compatible API.
+//! Docs: https://docs.ai21.com/reference/jamba-instruct-api
+
+use async_trait::async_trait;
+use crate::openai::OpenAiProvider;
+use astrbot_core::provider::{Provider, ChatMessage, ChatConfig, ChatResponse, ChatStreamChunk, ModelInfo};
+use astrbot_core::errors::Result;
+use futures_util::Stream;
+
+/// AI21 provider wrapper
+pub struct Ai21Provider {
+    inner: OpenAiProvider,
+}
+
+impl Ai21Provider {
+    pub fn new(id: String, api_key: String, model: String) -> Self {
+        let base_url = "https://api.ai21.com/studio/v1".to_string();
+        Self {
+            inner: OpenAiProvider::new(id, api_key, base_url, model),
+        }
+    }
+}
+
+#[async_trait]
+impl Provider for Ai21Provider {
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
+    fn test_ai21_provider_creation() {
+        let p = Ai21Provider::new(
+            "ai21-1".to_string(),
+            "sk-ai21-test".to_string(),
+            "jamba-1.5-large".to_string(),
+        );
+        assert_eq!(p.id(), "ai21-1");
+        assert_eq!(p.name(), "ai21-1");
+    }
+}
