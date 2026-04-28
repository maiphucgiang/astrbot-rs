use crate::{OpenAiCompatibleProvider, ProviderConfig};

pub fn create(api_key: String, model: String) -> OpenAiCompatibleProvider {
    OpenAiCompatibleProvider::new(ProviderConfig {
        name: "Azure".to_string(),
        base_url: "https://api.azure.com/openai".to_string(),
        api_key,
        model,
        extra_headers: None,
    })
}
