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
    async fn synthesize(&self, _text: &str) -> Result<Bytes> {
        info!("OpenAiTts synthesize called (skeleton)");
        // Skeleton: will be wired to reqwest + /v1/audio/speech endpoint.
        Err(AstrBotError::NotImplemented(
            "OpenAiTts::synthesize not yet implemented".to_string(),
        ))
    }

    async fn health_check(&self) -> Result<()> {
        info!("OpenAiTts health_check called (skeleton)");
        // Skeleton: placeholder OK.
        Ok(())
    }
}
