use crate::errors::{AstrBotError, Result};
use async_trait::async_trait;
use bytes::Bytes;
use tracing::info;

// ---------------------------------------------------------------------------
// TTS Engine trait
// ---------------------------------------------------------------------------

/// Text-to-Speech engine.
/// Converts plain text into an audio byte stream.
#[async_trait]
pub trait TtsEngine: Send + Sync {
    /// Synthesize speech from text.
    /// Returns a PCM or encoded audio byte stream.
    async fn synthesize(&self, text: &str) -> Result<Bytes>;

    /// Quick health check (e.g. ping endpoint or validate API key).
    async fn health_check(&self) -> Result<()>;
}

// ---------------------------------------------------------------------------
// OpenAI TTS implementation (skeleton)
// ---------------------------------------------------------------------------

/// OpenAI TTS engine — `tts-1` model.
pub struct OpenAiTts {
    #[allow(dead_code)]
    base_url: String,
    #[allow(dead_code)]
    api_key: String,
    #[allow(dead_code)]
    model: String,
    #[allow(dead_code)]
    voice: String,
}

impl OpenAiTts {
    pub fn new(base_url: String, api_key: String) -> Self {
        Self {
            base_url,
            api_key,
            model: "tts-1".to_string(),
            voice: "alloy".to_string(),
        }
    }

    pub fn with_model(mut self, model: String) -> Self {
        self.model = model;
        self
    }

    pub fn with_voice(mut self, voice: String) -> Self {
        self.voice = voice;
        self
    }
}

#[async_trait]
impl TtsEngine for OpenAiTts {
    async fn synthesize(&self, text: &str) -> Result<Bytes> {
        info!("[OpenAiTts] synthesize — {} chars", text.len());

        let client = reqwest::Client::new();
        let url = format!("{}/v1/audio/speech", self.base_url);

        let body = serde_json::json!({
            "model": self.model,
            "input": text,
            "voice": self.voice,
        });

        let resp = client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| AstrBotError::Network(format!("TTS request failed: {}", e)))?;

        let status = resp.status();
        if !status.is_success() {
            let text_err = resp.text().await.unwrap_or_default();
            return Err(AstrBotError::Network(format!(
                "TTS HTTP {}: {}",
                status, text_err
            )));
        }

        let audio = resp
            .bytes()
            .await
            .map_err(|e| AstrBotError::Network(format!("TTS audio read: {}", e)))?;

        Ok(audio)
    }

    async fn health_check(&self) -> Result<()> {
        let client = reqwest::Client::new();
        let url = format!("{}/v1/models", self.base_url);
        let resp = client
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .send()
            .await
            .map_err(|e| AstrBotError::Network(format!("TTS health check: {}", e)))?;

        if resp.status().is_success() {
            Ok(())
        } else {
            Err(AstrBotError::Network(format!(
                "TTS health check failed: HTTP {}",
                resp.status()
            )))
        }
    }
}
