use crate::{OpenAiCompatibleProvider, ProviderConfig};

pub fn create(api_key: String, model: String) -> OpenAiCompatibleProvider {
    OpenAiCompatibleProvider::new(ProviderConfig {
        name: "Together".to_string(),
        base_url: "https://api.together.xyz/v1".to_string(),
        api_key,
        model,
        extra_headers: None,
    })
}
