use crate::{OpenAiCompatibleProvider, ProviderConfig};

pub fn create(api_key: String, model: String) -> OpenAiCompatibleProvider {
    OpenAiCompatibleProvider::new(ProviderConfig {
        name: "Zhipu AI".to_string(),
        base_url: "https://open.bigmodel.cn/api/paas/v4".to_string(),
        api_key: api_key.clone(),
        model,
        extra_headers: Some(vec![
            ("Authorization".to_string(), format!("Bearer {}", api_key)),
        ]),
    })
}
