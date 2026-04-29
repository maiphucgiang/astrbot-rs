use crate::{OpenAiCompatibleProvider, ProviderConfig};

pub fn create(api_key: String, model: String) -> OpenAiCompatibleProvider {
    OpenAiCompatibleProvider::new(ProviderConfig {
        name: "Tencent Hunyuan".to_string(),
        base_url: "https://hunyuan.tencentcloudapi.com/compatible-mode/v1".to_string(),
        api_key,
        model,
        extra_headers: None,
    })
}
