use crate::{OpenAiCompatibleProvider, ProviderConfig};

pub fn create(api_key: String, model: String) -> OpenAiCompatibleProvider {
    OpenAiCompatibleProvider::new(ProviderConfig {
        name: "Perplexity".to_string(),
        base_url: "https://api.perplexity.ai".to_string(),
        api_key,
        model,
        extra_headers: None,
    })
}
