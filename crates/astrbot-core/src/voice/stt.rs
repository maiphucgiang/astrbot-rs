use crate::errors::{AstrBotError, Result};
use crate::provider::SttProvider;
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

// ---------------------------------------------------------------------------
// SttProvider trait bridge
// ---------------------------------------------------------------------------

#[async_trait]
impl SttProvider for OpenAiWhisper {
    fn id(&self) -> &str {
        "openai_whisper"
    }
    fn name(&self) -> &str {
        "OpenAI Whisper"
    }
    async fn transcribe(&self, audio: Bytes) -> Result<String> {
        SttEngine::transcribe(self, audio).await
    }
    async fn health_check(&self) -> Result<()> {
        SttEngine::health_check(self).await
    }
    fn supported_formats(&self) -> Vec<String> {
        vec!["wav".to_string(), "mp3".to_string(), "m4a".to_string(), "ogg".to_string()]
    }
}

// ---------------------------------------------------------------------------
// SenseVoice STT implementation (local inference via subprocess)
// ---------------------------------------------------------------------------

pub struct SenseVoiceStt {
    model_path: String,
    python_path: String,
}

impl SenseVoiceStt {
    pub fn new(model_path: String, python_path: String) -> Self {
        Self { model_path, python_path }
    }
}

#[async_trait]
impl SttEngine for SenseVoiceStt {
    async fn transcribe(&self, audio: Bytes) -> Result<String> {
        info!("[SenseVoiceStt] transcribe — {} bytes", audio.len());
        // Write audio to temp file
        let temp_dir = std::env::temp_dir();
        let temp_path = temp_dir.join(format!("sensevoice_{}.wav", uuid::Uuid::new_v4()));
        tokio::fs::write(&temp_path, audio)
            .await
            .map_err(|e| AstrBotError::Internal(format!("Temp write: {}", e)))?;
        // Spawn python process for inference
        let output = tokio::process::Command::new(&self.python_path)
            .arg("-c")
            .arg(format!(
                "from funasr import AutoModel; m=AutoModel(model='{}'); print(m.generate(input='{}'))",
                self.model_path, temp_path.display()
            ))
            .output()
            .await
            .map_err(|e| AstrBotError::Internal(format!("SenseVoice subprocess: {}", e)))?;
        // Cleanup
        let _ = tokio::fs::remove_file(&temp_path).await;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(AstrBotError::Internal(format!("SenseVoice failed: {}", stderr)));
        }
        let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if text.is_empty() {
            return Err(AstrBotError::Internal("SenseVoice returned empty".to_string()));
        }
        Ok(text)
    }
    async fn health_check(&self) -> Result<()> {
        let output = tokio::process::Command::new(&self.python_path)
            .arg("-c")
            .arg("import funasr; print('ok')")
            .output()
            .await
            .map_err(|e| AstrBotError::Internal(format!("SenseVoice health: {}", e)))?;
        if output.status.success() {
            Ok(())
        } else {
            Err(AstrBotError::Internal("SenseVoice dependency missing".to_string()))
        }
    }
}

#[async_trait]
impl SttProvider for SenseVoiceStt {
    fn id(&self) -> &str {
        "sensevoice"
    }
    fn name(&self) -> &str {
        "SenseVoice (Local)"
    }
    async fn transcribe(&self, audio: Bytes) -> Result<String> {
        SttEngine::transcribe(self, audio).await
    }
    async fn health_check(&self) -> Result<()> {
        SttEngine::health_check(self).await
    }
    fn supported_formats(&self) -> Vec<String> {
        vec!["wav".to_string(), "mp3".to_string(), "pcm".to_string()]
    }
}
