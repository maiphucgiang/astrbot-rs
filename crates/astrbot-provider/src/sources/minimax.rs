use crate::{OpenAiCompatibleProvider, ProviderConfig};

pub fn create(api_key: String, model: String) -> OpenAiCompatibleProvider {
    OpenAiCompatibleProvider::new(ProviderConfig {
        name: "MiniMax".to_string(),
        base_url: "https://api.minimax.chat/v1".to_string(),
        api_key,
        model,
        extra_headers: None,
    })
}
