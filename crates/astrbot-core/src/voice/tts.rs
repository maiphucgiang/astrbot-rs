use crate::errors::{AstrBotError, Result};
use crate::provider::TtsProvider;
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
        // Skeleton: no-op health check
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// TtsProvider trait bridge
// ---------------------------------------------------------------------------

#[async_trait]
impl TtsProvider for OpenAiTts {
    fn id(&self) -> &str {
        "openai_tts"
    }
    fn name(&self) -> &str {
        "OpenAI TTS"
    }
    async fn synthesize(&self, text: &str) -> Result<Bytes> {
        TtsEngine::synthesize(self, text).await
    }
    async fn health_check(&self) -> Result<()> {
        TtsEngine::health_check(self).await
    }
    async fn voices(&self) -> Result<Vec<String>> {
        Ok(vec![
            "alloy".to_string(),
            "echo".to_string(),
            "fable".to_string(),
            "onyx".to_string(),
            "nova".to_string(),
            "shimmer".to_string(),
        ])
    }
}

// ---------------------------------------------------------------------------
// Azure TTS implementation
// ---------------------------------------------------------------------------

pub struct AzureTts {
    region: String,
    api_key: String,
    voice: String,
}

impl AzureTts {
    pub fn new(region: String, api_key: String) -> Self {
        Self {
            region,
            api_key,
            voice: "en-US-JennyNeural".to_string(),
        }
    }
    pub fn with_voice(mut self, voice: String) -> Self {
        self.voice = voice;
        self
    }
}

#[async_trait]
impl TtsEngine for AzureTts {
    async fn synthesize(&self, text: &str) -> Result<Bytes> {
        info!("[AzureTts] synthesize — {} chars", text.len());
        let client = reqwest::Client::new();
        let url = format!(
            "https://{}.tts.speech.microsoft.com/cognitiveservices/v1",
            self.region
        );
        let body = format!(
            "<speak version='1.0' xml:lang='en-US'><voice xml:lang='en-US' name='{}'>{}</voice></speak>",
            self.voice, text
        );
        let resp = client
            .post(&url)
            .header("Ocp-Apim-Subscription-Key", &self.api_key)
            .header("Content-Type", "application/ssml+xml")
            .header(
                "X-Microsoft-OutputFormat",
                "audio-24khz-160kbitrate-mono-mp3",
            )
            .body(body)
            .send()
            .await
            .map_err(|e| AstrBotError::Network(format!("Azure TTS request: {}", e)))?;
        let status = resp.status();
        if !status.is_success() {
            let err = resp.text().await.unwrap_or_default();
            return Err(AstrBotError::Network(format!(
                "Azure TTS HTTP {}: {}",
                status, err
            )));
        }
        let audio = resp
            .bytes()
            .await
            .map_err(|e| AstrBotError::Network(format!("Azure TTS audio read: {}", e)))?;
        Ok(audio)
    }
    async fn health_check(&self) -> Result<()> {
        let client = reqwest::Client::new();
        let url = format!(
            "https://{}.api.cognitive.microsoft.com/sts/v1.0/issueToken",
            self.region
        );
        let resp = client
            .post(&url)
            .header("Ocp-Apim-Subscription-Key", &self.api_key)
            .header("Content-Length", "0")
            .send()
            .await
            .map_err(|e| AstrBotError::Network(format!("Azure TTS health: {}", e)))?;
        if resp.status().is_success() {
            Ok(())
        } else {
            Err(AstrBotError::Network(format!(
                "Azure TTS health failed: HTTP {}",
                resp.status()
            )))
        }
    }
}

#[async_trait]
impl TtsProvider for AzureTts {
    fn id(&self) -> &str {
        "azure_tts"
    }
    fn name(&self) -> &str {
        "Azure TTS"
    }
    async fn synthesize(&self, text: &str) -> Result<Bytes> {
        TtsEngine::synthesize(self, text).await
    }
    async fn health_check(&self) -> Result<()> {
        TtsEngine::health_check(self).await
    }
    async fn voices(&self) -> Result<Vec<String>> {
        // Skeleton: return a static list
        Ok(vec![self.voice.clone()])
    }
}

// ---------------------------------------------------------------------------
// Edge TTS implementation (Microsoft Edge Read Aloud)
// ---------------------------------------------------------------------------

pub struct EdgeTts {
    voice: String,
}

impl EdgeTts {
    pub fn new() -> Self {
        Self {
            voice: "zh-CN-XiaoxiaoNeural".to_string(),
        }
    }
    pub fn with_voice(mut self, voice: String) -> Self {
        self.voice = voice;
        self
    }
}

#[async_trait]
impl TtsEngine for EdgeTts {
    async fn synthesize(&self, text: &str) -> Result<Bytes> {
        info!("[EdgeTts] synthesize — {} chars", text.len());
        // Edge TTS uses WebSocket to speech.platform.bing.com
        // Skeleton: return a placeholder error for now
        Err(AstrBotError::Internal(
            "EdgeTTS WebSocket implementation pending".to_string(),
        ))
    }
    async fn health_check(&self) -> Result<()> {
        Ok(())
    }
}

#[async_trait]
impl TtsProvider for EdgeTts {
    fn id(&self) -> &str {
        "edge_tts"
    }
    fn name(&self) -> &str {
        "Edge TTS"
    }
    async fn synthesize(&self, text: &str) -> Result<Bytes> {
        TtsEngine::synthesize(self, text).await
    }
    async fn health_check(&self) -> Result<()> {
        TtsEngine::health_check(self).await
    }
    async fn voices(&self) -> Result<Vec<String>> {
        Ok(vec![self.voice.clone()])
    }
}

// ---------------------------------------------------------------------------
// FishAudio TTS implementation
// ---------------------------------------------------------------------------

pub struct FishAudioTts {
    #[allow(dead_code)]
    api_key: String,
    #[allow(dead_code)]
    model: String,
}

impl FishAudioTts {
    pub fn new(api_key: String) -> Self {
        Self {
            api_key,
            model: "default".to_string(),
        }
    }
    pub fn with_model(mut self, model: String) -> Self {
        self.model = model;
        self
    }
}

#[async_trait]
impl TtsEngine for FishAudioTts {
    async fn synthesize(&self, _text: &str) -> Result<Bytes> {
        info!("[FishAudioTts] synthesize — skeleton");
        Err(AstrBotError::Internal(
            "FishAudio TTS skeleton — not yet implemented".to_string(),
        ))
    }
    async fn health_check(&self) -> Result<()> {
        let client = reqwest::Client::new();
        let resp = client
            .get("https://api.fish.audio/v1/models")
            .header("Authorization", format!("Bearer {}", self.api_key))
            .send()
            .await
            .map_err(|e| AstrBotError::Network(format!("FishAudio health: {}", e)))?;
        if resp.status().is_success() {
            Ok(())
        } else {
            Err(AstrBotError::Network(format!(
                "FishAudio health failed: HTTP {}",
                resp.status()
            )))
        }
    }
}

#[async_trait]
impl TtsProvider for FishAudioTts {
    fn id(&self) -> &str {
        "fishaudio_tts"
    }
    fn name(&self) -> &str {
        "FishAudio TTS"
    }
    async fn synthesize(&self, text: &str) -> Result<Bytes> {
        TtsEngine::synthesize(self, text).await
    }
    async fn health_check(&self) -> Result<()> {
        TtsEngine::health_check(self).await
    }
    async fn voices(&self) -> Result<Vec<String>> {
        Ok(vec![self.model.clone()])
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_openai_tts_voices() {
        let tts = OpenAiTts::new("https://api.openai.com".into(), "sk-test".into());
        let voices = tts.voices().await.unwrap();
        assert_eq!(voices.len(), 6);
        assert!(voices.contains(&"alloy".to_string()));
    }

    #[tokio::test]
    async fn test_azure_tts_voices() {
        let tts =
            AzureTts::new("westus".into(), "key".into()).with_voice("zh-CN-XiaoxiaoNeural".into());
        let voices = tts.voices().await.unwrap();
        assert!(voices.contains(&"zh-CN-XiaoxiaoNeural".to_string()));
    }

    #[tokio::test]
    async fn test_edge_tts_voices() {
        let tts = EdgeTts::new().with_voice("en-US-AriaNeural".into());
        let voices = tts.voices().await.unwrap();
        assert_eq!(voices, vec!["en-US-AriaNeural".to_string()]);
    }

    #[tokio::test]
    async fn test_fishaudio_tts_voices() {
        let tts = FishAudioTts::new("fake_key".into()).with_model("model_01".into());
        let voices = tts.voices().await.unwrap();
        assert_eq!(voices, vec!["model_01".to_string()]);
    }

    #[tokio::test]
    async fn test_fishaudio_tts_skeleton_error() {
        let tts = FishAudioTts::new("fake_key".into());
        let result = TtsEngine::synthesize(&tts, "Hello").await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("FishAudio") || err.contains("skeleton"));
    }

    #[tokio::test]
    async fn test_tts_engine_trait_object() {
        let tts: Box<dyn TtsEngine> = Box::new(OpenAiTts::new(
            "https://api.openai.com".into(),
            "sk-test".into(),
        ));
        let result = tts.synthesize("test").await;
        assert!(result.is_err());
    }
}
