use async_trait::async_trait;
use futures::{SinkExt, Stream, StreamExt};
use reqwest::Client;
use serde_json::Value;
use std::pin::Pin;
use std::task::{Context, Poll};

use crate::{ChatMessage, ChatOptions, ChatProvider, ProviderConfig, ProviderError};

/// SSE 流式响应解析器
struct SseStream {
    inner: Pin<Box<dyn Stream<Item = Result<bytes::Bytes, reqwest::Error>> + Send>>,
    buffer: String,
}

impl SseStream {
    fn new(response: reqwest::Response) -> Self {
        Self {
            inner: Box::pin(response.bytes_stream()),
            buffer: String::new(),
        }
    }
}

impl Stream for SseStream {
    type Item = Result<String, ProviderError>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        loop {
            match self.inner.as_mut().poll_next(cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(None) => return Poll::Ready(None),
                Poll::Ready(Some(Ok(bytes))) => {
                    self.buffer.push_str(&String::from_utf8_lossy(&bytes));
                    while let Some(pos) = self.buffer.find('\n') {
                        let line = self.buffer.drain(..=pos).collect::<String>();
                        if let Some(data) = line.strip_prefix("data: ") {
                            let data = data.trim();
                            if data == "[DONE]" {
                                return Poll::Ready(None);
                            }
                            if let Ok(json) = serde_json::from_str::<Value>(data) {
                                if let Some(content) =
                                    json["choices"][0]["delta"]["content"].as_str()
                                {
                                    if !content.is_empty() {
                                        return Poll::Ready(Some(Ok(content.to_string())));
                                    }
                                }
                            }
                        }
                    }
                }
                Poll::Ready(Some(Err(e))) => {
                    return Poll::Ready(Some(Err(ProviderError::Http(e))));
                }
            }
        }
    }
}

/// OpenAI API 兼容的 Provider 通用实现
pub struct OpenAiCompatibleProvider {
    client: Client,
    config: ProviderConfig,
}

impl OpenAiCompatibleProvider {
    pub fn new(config: ProviderConfig) -> Self {
        Self {
            client: Client::new(),
            config,
        }
    }

    pub fn config(&self) -> &ProviderConfig {
        &self.config
    }

    /// 构建 HTTP 请求的基础方法
    fn build_request(&self, endpoint: &str, body: serde_json::Value) -> reqwest::RequestBuilder {
        let url = format!("{}/{}", self.config.base_url.trim_end_matches('/'), endpoint);
        let mut req = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.config.api_key))
            .header("Content-Type", "application/json")
            .json(&body);

        if let Some(headers) = &self.config.extra_headers {
            for (k, v) in headers {
                req = req.header(k, v);
            }
        }
        req
    }
}

#[async_trait]
impl ChatProvider for OpenAiCompatibleProvider {
    fn name(&self) -> &str {
        &self.config.name
    }

    async fn chat(
        &self,
        messages: Vec<ChatMessage>,
        options: ChatOptions,
    ) -> Result<String, ProviderError> {
        let model = options.model.as_ref().unwrap_or(&self.config.model);

        let body = serde_json::json!({
            "model": model,
            "messages": messages,
            "temperature": options.temperature.unwrap_or(0.7),
            "max_tokens": options.max_tokens,
            "top_p": options.top_p.unwrap_or(1.0),
        });

        let response = self.build_request("v1/chat/completions", body).send().await?;
        let status = response.status();

        if !status.is_success() {
            let text = response.text().await.unwrap_or_default();
            return Err(ProviderError::Api {
                status: status.as_u16(),
                message: text,
            });
        }

        let json: Value = response.json().await?;
        let content = json["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or("")
            .to_string();

        Ok(content)
    }

    fn supports_streaming(&self) -> bool {
        true
    }

    async fn stream_chat(
        &self,
        messages: Vec<ChatMessage>,
        options: ChatOptions,
    ) -> Result<Box<dyn futures::Stream<Item = Result<String, ProviderError>> + Send>, ProviderError> {
        let model = options.model.as_ref().unwrap_or(&self.config.model);

        let body = serde_json::json!({
            "model": model,
            "messages": messages,
            "temperature": options.temperature.unwrap_or(0.7),
            "max_tokens": options.max_tokens,
            "top_p": options.top_p.unwrap_or(1.0),
            "stream": true,
        });

        let response = self.build_request("v1/chat/completions", body).send().await?;
        let status = response.status();

        if !status.is_success() {
            let text = response.text().await.unwrap_or_default();
            return Err(ProviderError::Api {
                status: status.as_u16(),
                message: text,
            });
        }

        Ok(Box::new(SseStream::new(response)))
    }

    fn list_models(&self) -> Vec<String> {
        vec![self.config.model.clone()]
    }

    fn is_available(&self) -> bool {
        !self.config.api_key.is_empty()
    }
}

#[async_trait]
impl crate::EmbeddingProvider for OpenAiCompatibleProvider {
    fn name(&self) -> &str {
        &self.config.name
    }

    async fn embed(&self, texts: Vec<String>) -> Result<Vec<Vec<f32>>, ProviderError> {
        let body = serde_json::json!({
            "model": self.config.model,
            "input": texts,
        });

        let response = self.build_request("v1/embeddings", body).send().await?;
        let status = response.status();

        if !status.is_success() {
            let text = response.text().await.unwrap_or_default();
            return Err(ProviderError::Api {
                status: status.as_u16(),
                message: text,
            });
        }

        let json: Value = response.json().await?;
        let embeddings: Vec<Vec<f32>> = json["data"]
            .as_array()
            .unwrap_or(&Vec::new())
            .iter()
            .map(|item| {
                item["embedding"]
                    .as_array()
                    .unwrap_or(&Vec::new())
                    .iter()
                    .map(|v| v.as_f64().unwrap_or(0.0) as f32)
                    .collect()
            })
            .collect();

        Ok(embeddings)
    }
}
