use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

pub mod sender;
pub mod stt;
pub mod tts;

pub use stt::{AzureStt, OpenAiWhisper, SenseVoiceStt, SttEngine};
pub use tts::{AzureTts, EdgeTts, FishAudioTts, OpenAiTts, TtsEngine};

// ---------------------------------------------------------------------------
// Voice Registry
// ---------------------------------------------------------------------------

/// Unified registry for TTS and STT engines.
pub struct VoiceRegistry {
    tts_engines: RwLock<HashMap<String, Arc<dyn TtsEngine>>>,
    stt_engines: RwLock<HashMap<String, Arc<dyn SttEngine>>>,
}

impl Default for VoiceRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl VoiceRegistry {
    pub fn new() -> Self {
        Self {
            tts_engines: RwLock::new(HashMap::new()),
            stt_engines: RwLock::new(HashMap::new()),
        }
    }

    // --- TTS ---

    pub async fn register_tts(&self, name: String, engine: Arc<dyn TtsEngine>) {
        let mut map = self.tts_engines.write().await;
        map.insert(name, engine);
    }

    pub async fn unregister_tts(&self, name: &str) {
        let mut map = self.tts_engines.write().await;
        map.remove(name);
    }

    pub async fn get_tts(&self, name: &str) -> Option<Arc<dyn TtsEngine>> {
        let map = self.tts_engines.read().await;
        map.get(name).cloned()
    }

    pub async fn list_tts(&self) -> Vec<String> {
        let map = self.tts_engines.read().await;
        map.keys().cloned().collect()
    }

    // --- STT ---

    pub async fn register_stt(&self, name: String, engine: Arc<dyn SttEngine>) {
        let mut map = self.stt_engines.write().await;
        map.insert(name, engine);
    }

    pub async fn unregister_stt(&self, name: &str) {
        let mut map = self.stt_engines.write().await;
        map.remove(name);
    }

    pub async fn get_stt(&self, name: &str) -> Option<Arc<dyn SttEngine>> {
        let map = self.stt_engines.read().await;
        map.get(name).cloned()
    }

    pub async fn list_stt(&self) -> Vec<String> {
        let map = self.stt_engines.read().await;
        map.keys().cloned().collect()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;

    #[tokio::test]
    async fn test_tts_registry() {
        let registry = VoiceRegistry::new();
        let tts = Arc::new(OpenAiTts::new(
            "https://api.openai.com".into(),
            "sk-test".into(),
        ));
        registry.register_tts("openai".to_string(), tts).await;

        let list = registry.list_tts().await;
        assert_eq!(list.len(), 1);
        assert!(registry.get_tts("openai").await.is_some());
        assert!(registry.get_tts("nonexistent").await.is_none());

        registry.unregister_tts("openai").await;
        assert!(registry.get_tts("openai").await.is_none());
    }

    #[tokio::test]
    async fn test_stt_registry() {
        let registry = VoiceRegistry::new();
        let stt = Arc::new(OpenAiWhisper::new(
            "https://api.openai.com".into(),
            "sk-test".into(),
        ));
        registry.register_stt("whisper".to_string(), stt).await;

        let list = registry.list_stt().await;
        assert_eq!(list.len(), 1);
        assert!(registry.get_stt("whisper").await.is_some());
        assert!(registry.get_stt("nonexistent").await.is_none());

        registry.unregister_stt("whisper").await;
        assert!(registry.get_stt("whisper").await.is_none());
    }

    #[tokio::test]
    async fn test_tts_synthesize_skeleton() {
        let tts = OpenAiTts::new("https://api.openai.com".into(), "sk-test".into());
        // The skeleton returns an error; assert it's an error rather than panicking.
        let result = tts.synthesize("Hello, world!").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_stt_transcribe_skeleton() {
        let stt = OpenAiWhisper::new("https://api.openai.com".into(), "sk-test".into());
        let audio = Bytes::from_static(b"fake_audio_data");
        let result = stt.transcribe(audio).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_tts_health_check_skeleton() {
        let tts = OpenAiTts::new("https://api.openai.com".into(), "sk-test".into());
        let result = tts.health_check().await;
        // Skeleton returns Ok(()) as a placeholder.
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_stt_health_check_skeleton() {
        let stt = OpenAiWhisper::new("https://api.openai.com".into(), "sk-test".into());
        let result = stt.health_check().await;
        assert!(result.is_ok());
    }
}
