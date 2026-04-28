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
    async fn transcribe(&self, _audio: Bytes) -> Result<String> {
        info!("OpenAiWhisper transcribe called (skeleton)");
        // Skeleton: will be wired to reqwest + /v1/audio/transcriptions endpoint.
        Err(AstrBotError::NotImplemented(
            "OpenAiWhisper::transcribe not yet implemented".to_string(),
        ))
    }

    async fn health_check(&self) -> Result<()> {
        info!("OpenAiWhisper health_check called (skeleton)");
        // Skeleton: placeholder OK.
        Ok(())
    }
}
