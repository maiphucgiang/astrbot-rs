use crate::{OpenAiCompatibleProvider, ProviderConfig};

pub fn create(api_key: String, model: String) -> OpenAiCompatibleProvider {
    OpenAiCompatibleProvider::new(ProviderConfig {
        name: "01.AI".to_string(),
        base_url: "https://api.01.ai/v1".to_string(),
        api_key,
        model,
        extra_headers: None,
    })
}
