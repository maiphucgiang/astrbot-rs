use crate::errors::{AstrBotError, Result};
use crate::provider::SttProvider;
use async_trait::async_trait;
use bytes::Bytes;
use tracing::info;

// ---------------------------------------------------------------------------
// STT Engine trait
// ---------------------------------------------------------------------------

#[async_trait]
pub trait SttEngine: Send + Sync {
    async fn transcribe(&self, audio: Bytes) -> Result<String>;
    async fn health_check(&self) -> Result<()>;
}

// ---------------------------------------------------------------------------
// OpenAI Whisper
// ---------------------------------------------------------------------------

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
        info!("[OpenAiWhisper] health_check — skeleton");
        Ok(())
    }
}

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
        vec!["wav".into(), "mp3".into(), "m4a".into(), "ogg".into()]
    }
}

// ---------------------------------------------------------------------------
// Azure STT
// ---------------------------------------------------------------------------

pub struct AzureStt {
    region: String,
    api_key: String,
    language: String,
}

impl AzureStt {
    pub fn new(region: String, api_key: String) -> Self {
        Self {
            region,
            api_key,
            language: "en-US".to_string(),
        }
    }
    pub fn with_language(mut self, language: String) -> Self {
        self.language = language;
        self
    }
}

#[async_trait]
impl SttEngine for AzureStt {
    async fn transcribe(&self, audio: Bytes) -> Result<String> {
        info!("[AzureStt] transcribe — {} bytes", audio.len());
        let client = reqwest::Client::new();
        let url = if self.region.starts_with("http://") || self.region.starts_with("https://") {
            format!(
                "{}/speech/recognition/conversation/cognitiveservices/v1?language={}",
                self.region.trim_end_matches('/'),
                self.language
            )
        } else {
            format!(
                "https://{}.stt.speech.microsoft.com/speech/recognition/conversation/cognitiveservices/v1?language={}",
                self.region, self.language
            )
        };
        let resp = client
            .post(&url)
            .header("Ocp-Apim-Subscription-Key", &self.api_key)
            .header(
                "Content-Type",
                "audio/wav; codecs=audio/pcm; samplerate=16000",
            )
            .header("Accept", "application/json")
            .body(Vec::from(audio))
            .send()
            .await
            .map_err(|e| AstrBotError::Network(format!("Azure STT request: {}", e)))?;
        let status = resp.status();
        if !status.is_success() {
            let err = resp.text().await.unwrap_or_default();
            return Err(AstrBotError::Network(format!(
                "Azure STT HTTP {}: {}",
                status, err
            )));
        }
        let json: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| AstrBotError::Serialization(format!("Azure STT JSON parse: {}", e)))?;
        let text = json
            .get("DisplayText")
            .and_then(|v| v.as_str())
            .or_else(|| {
                json.get("NBest")
                    .and_then(|a| a.get(0))
                    .and_then(|o| o.get("Display"))
                    .and_then(|v| v.as_str())
            })
            .unwrap_or("")
            .to_string();
        if text.is_empty() {
            return Err(AstrBotError::Internal(
                "Azure STT returned empty transcription".to_string(),
            ));
        }
        Ok(text)
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
            .map_err(|e| AstrBotError::Network(format!("Azure STT health: {}", e)))?;
        if resp.status().is_success() {
            Ok(())
        } else {
            Err(AstrBotError::Network(format!(
                "Azure STT health failed: HTTP {}",
                resp.status()
            )))
        }
    }
}

#[async_trait]
impl SttProvider for AzureStt {
    fn id(&self) -> &str {
        "azure_stt"
    }
    fn name(&self) -> &str {
        "Azure STT"
    }
    async fn transcribe(&self, audio: Bytes) -> Result<String> {
        SttEngine::transcribe(self, audio).await
    }
    async fn health_check(&self) -> Result<()> {
        SttEngine::health_check(self).await
    }
    fn supported_formats(&self) -> Vec<String> {
        vec!["wav".into(), "mp3".into(), "ogg".into(), "flac".into()]
    }
}

// ---------------------------------------------------------------------------
// SenseVoice (Local)
// ---------------------------------------------------------------------------

pub struct SenseVoiceStt {
    model_path: String,
    python_path: String,
}

impl SenseVoiceStt {
    pub fn new(model_path: String, python_path: String) -> Self {
        Self {
            model_path,
            python_path,
        }
    }
}

#[async_trait]
impl SttEngine for SenseVoiceStt {
    async fn transcribe(&self, audio: Bytes) -> Result<String> {
        info!("[SenseVoiceStt] transcribe — {} bytes", audio.len());
        let temp_dir = std::env::temp_dir();
        let temp_path = temp_dir.join(format!("sensevoice_{}.wav", uuid::Uuid::new_v4()));
        tokio::fs::write(&temp_path, audio)
            .await
            .map_err(|e| AstrBotError::Internal(format!("Temp write: {}", e)))?;
        let output = tokio::process::Command::new(&self.python_path)
            .arg("-c")
            .arg(format!(
                "from funasr import AutoModel; m=AutoModel(model='{}'); print(m.generate(input='{}'))",
                self.model_path, temp_path.display()
            ))
            .output()
            .await
            .map_err(|e| AstrBotError::Internal(format!("SenseVoice subprocess: {}", e)))?;
        let _ = tokio::fs::remove_file(&temp_path).await;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(AstrBotError::Internal(format!(
                "SenseVoice failed: {}",
                stderr
            )));
        }
        let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if text.is_empty() {
            return Err(AstrBotError::Internal(
                "SenseVoice returned empty".to_string(),
            ));
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
            Err(AstrBotError::Internal(
                "SenseVoice dependency missing".to_string(),
            ))
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
        vec!["wav".into(), "mp3".into(), "pcm".into()]
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    async fn run_mock_http_server(response_body: &'static str) -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buf = vec![0u8; 1024];
            let _ = socket.read(&mut buf).await.unwrap();
            let http_response = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/json\r\n\r\n{}",
                response_body.len(),
                response_body
            );
            let _ = socket.write_all(http_response.as_bytes()).await;
        });
        port
    }

    #[tokio::test]
    async fn test_whisper_transcribe_mock() {
        let body = r#"{"text":"Hello world"}"#;
        let port = run_mock_http_server(body).await;
        let whisper = OpenAiWhisper {
            base_url: format!("http://127.0.0.1:{}", port),
            api_key: "test_key".into(),
            model: "whisper-1".into(),
        };
        let text = SttEngine::transcribe(&whisper, Bytes::from_static(b"fake_audio"))
            .await
            .unwrap();
        assert_eq!(text, "Hello world");
    }

    #[tokio::test]
    async fn test_whisper_voices_list() {
        let formats = OpenAiWhisper::new("https://api.openai.com".into(), "sk-test".into())
            .supported_formats();
        assert!(formats.contains(&"wav".to_string()));
        assert!(formats.contains(&"mp3".to_string()));
    }

    #[tokio::test]
    async fn test_azure_stt_transcribe_mock() {
        let body = r#"{"DisplayText":"Good morning"}"#;
        let port = run_mock_http_server(body).await;
        let stt = AzureStt {
            region: format!("http://127.0.0.1:{}", port),
            api_key: "test_key".into(),
            language: "en-US".into(),
        };
        let text = SttEngine::transcribe(&stt, Bytes::from_static(b"fake_audio"))
            .await
            .unwrap();
        assert_eq!(text, "Good morning");
    }

    #[tokio::test]
    async fn test_azure_stt_voices_list() {
        let formats = AzureStt::new("westus".into(), "key".into())
            .with_language("zh-CN".into())
            .supported_formats();
        assert!(formats.contains(&"wav".to_string()));
        assert!(formats.contains(&"flac".to_string()));
    }

    #[tokio::test]
    async fn test_sensevoice_health_check_failure() {
        let stt = SenseVoiceStt::new("/tmp/model".into(), "python3".into());
        assert!(SttEngine::health_check(&stt).await.is_err());
    }

    #[tokio::test]
    async fn test_sensevoice_formats() {
        let formats = SenseVoiceStt::new("/tmp/model".into(), "python3".into()).supported_formats();
        assert_eq!(formats, vec!["wav", "mp3", "pcm"]);
    }
}
