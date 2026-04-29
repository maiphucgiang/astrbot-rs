use crate::{OpenAiCompatibleProvider, ProviderConfig};

pub fn create(api_key: String, model: String) -> OpenAiCompatibleProvider {
    OpenAiCompatibleProvider::new(ProviderConfig {
        name: "OpenRouter".to_string(),
        base_url: "https://openrouter.ai/api/v1".to_string(),
        api_key,
        model,
        extra_headers: Some(vec![
            (
                "HTTP-Referer".to_string(),
                "https://astrbot.com".to_string(),
            ),
            ("X-Title".to_string(), "AstrBot".to_string()),
        ]),
    })
}
