use crate::errors::{AstrBotError, Result};
use async_trait::async_trait;
use bytes::Bytes;
use tracing::info;

// ---------------------------------------------------------------------------
// STT Engine trait
// ---------------------------------------------------------------------------

/// Speech-to-Text engine.
/// Converts an audio byte stream into transcribed text.
#[async_trait]
pub trait SttEngine: Send + Sync {
    /// Transcribe audio bytes into text.
    /// Accepts PCM, WAV, MP3, etc. — engine-specific.
    async fn transcribe(&self, audio: Bytes) -> Result<String>;

    /// Quick health check (e.g. ping endpoint or validate API key).
    async fn health_check(&self) -> Result<()>;
}

// ---------------------------------------------------------------------------
// OpenAI Whisper implementation (skeleton)
// ---------------------------------------------------------------------------

/// OpenAI Whisper STT engine — `whisper-1` model.
pub struct OpenAiWhisper {
    #[allow(dead_code)]
    base_url: String,
    #[allow(dead_code)]
    api_key: String,
    #[allow(dead_code)]
    model: String,
}

impl OpenAiWhisper {
    pub fn new(base_url: String, api_key: String) -> Self {
        Self {
            base_url,
            api_key,
            model: "whisper-1".to_string(),
        }
    }

    pub fn with_model(mut self, model: String) -> Self {
        self.model = model;
        self
    }
}

#[async_trait]
impl SttEngine for OpenAiWhisper {
    async fn transcribe(&self, audio: Bytes) -> Result<String> {
        info!("[OpenAiWhisper] transcribe — {} bytes", audio.len());

        let client = reqwest::Client::new();
        let url = format!("{}/v1/audio/transcriptions", self.base_url);

        let part = reqwest::multipart::Part::bytes(Vec::from(audio))
            .file_name("audio.wav")
            .mime_str("audio/wav")
            .map_err(|e| AstrBotError::Network(format!("multipart build: {}", e)))?;

        let form = reqwest::multipart::Form::new()
            .text("model", self.model.clone())
            .part("file", part);

        let resp = client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .multipart(form)
            .send()
            .await
            .map_err(|e| AstrBotError::Network(format!("Whisper request failed: {}", e)))?;

        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(AstrBotError::Network(format!(
                "Whisper HTTP {}: {}",
                status, text
            )));
        }

        let json: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| AstrBotError::Serialization(format!("Whisper JSON parse: {}", e)))?;

        let text = json
            .get("text")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        if text.is_empty() {
            return Err(AstrBotError::Internal(
                "Whisper returned empty transcription".to_string(),
            ));
        }

        Ok(text)
    }

    async fn health_check(&self) -> Result<()> {
        // Lightweight probe: check API key format and endpoint reachability
        let client = reqwest::Client::new();
        let url = format!("{}/v1/models", self.base_url);
        let resp = client
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .send()
            .await
            .map_err(|e| AstrBotError::Network(format!("Whisper health check: {}", e)))?;

        if resp.status().is_success() {
            Ok(())
        } else {
            Err(AstrBotError::Network(format!(
                "Whisper health check failed: HTTP {}",
                resp.status()
            )))
        }
    }
}
