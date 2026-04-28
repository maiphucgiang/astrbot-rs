use crate::{OpenAiCompatibleProvider, ProviderConfig};

pub fn create(api_key: String, model: String) -> OpenAiCompatibleProvider {
    OpenAiCompatibleProvider::new(ProviderConfig {
        name: "AI21".to_string(),
        base_url: "https://api.ai21.com/studio/v1".to_string(),
        api_key,
        model,
        extra_headers: None,
    })
}
