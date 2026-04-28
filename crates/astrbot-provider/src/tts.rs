use async_trait::async_trait;
use reqwest::Client;

use crate::ProviderError;

/// TTS Provider trait
#[async_trait]
pub trait TtsProvider: Send + Sync {
    fn name(&self) -> &str;
    async fn synthesize(&self, text: &str, voice: Option<&str>) -> Result<Vec<u8>, ProviderError>;
    fn supported_voices(&self) -> Vec<VoiceInfo>;
}

#[derive(Debug, Clone)]
pub struct VoiceInfo {
    pub id: String,
    pub name: String,
    pub language: Option<String>,
}

/// OpenAI-compatible TTS implementation
pub struct OpenAiCompatibleTts {
    client: Client,
    base_url: String,
    api_key: String,
    model: String,
    default_voice: String,
}

impl OpenAiCompatibleTts {
    pub fn new(base_url: String, api_key: String, model: String, default_voice: String) -> Self {
        Self {
            client: Client::new(),
            base_url,
            api_key,
            model,
            default_voice,
        }
    }
}

#[async_trait]
impl TtsProvider for OpenAiCompatibleTts {
    fn name(&self) -> &str {
        "OpenAI-Compatible TTS"
    }

    async fn synthesize(&self, text: &str, voice: Option<&str>) -> Result<Vec<u8>, ProviderError> {
        let body = serde_json::json!({
            "model": self.model,
            "input": text,
            "voice": voice.unwrap_or(&self.default_voice),
        });

        let response = self
            .client
            .post(format!("{}/v1/audio/speech", self.base_url.trim_end_matches('/')))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&body)
            .send()
            .await?;

        let bytes = response.bytes().await?.to_vec();
        Ok(bytes)
    }

    fn supported_voices(&self) -> Vec<VoiceInfo> {
        vec![]
    }
}
