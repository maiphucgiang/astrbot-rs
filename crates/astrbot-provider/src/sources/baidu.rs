use crate::{OpenAiCompatibleProvider, ProviderConfig};

pub fn create(api_key: String, model: String) -> OpenAiCompatibleProvider {
    OpenAiCompatibleProvider::new(ProviderConfig {
        name: "Baidu Qianfan".to_string(),
        base_url: "https://qianfan.baidubce.com/compatible-mode/v1".to_string(),
        api_key,
        model,
        extra_headers: None,
    })
}
