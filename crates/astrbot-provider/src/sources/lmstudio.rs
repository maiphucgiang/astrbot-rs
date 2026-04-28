use crate::{OpenAiCompatibleProvider, ProviderConfig};

pub fn create(base_url: String, model: String) -> OpenAiCompatibleProvider {
    OpenAiCompatibleProvider::new(ProviderConfig {
        name: "LM Studio".to_string(),
        base_url: format!("{}/v1", base_url.trim_end_matches('/')),
        api_key: "lm-studio".to_string(),
        model,
        extra_headers: None,
    })
}
