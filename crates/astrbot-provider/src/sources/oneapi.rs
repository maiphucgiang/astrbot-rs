use crate::{OpenAiCompatibleProvider, ProviderConfig};

pub fn create(base_url: String, api_key: String, model: String) -> OpenAiCompatibleProvider {
    OpenAiCompatibleProvider::new(ProviderConfig {
        name: "OneAPI".to_string(),
        base_url: format!("{}/v1", base_url.trim_end_matches('/')),
        api_key,
        model,
        extra_headers: None,
    })
}
