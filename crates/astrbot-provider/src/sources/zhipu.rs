use crate::{OpenAiCompatibleProvider, ProviderConfig};

/// 智谱 AI（GLM）Provider
/// OpenAI-compatible 封装，base_url: https://open.bigmodel.cn/api/paas/v4
/// 默认模型: glm-4
pub fn create(api_key: String, model: String) -> OpenAiCompatibleProvider {
    let model = if model.is_empty() { "glm-4".to_string() } else { model };
    OpenAiCompatibleProvider::new(ProviderConfig {
        name: "Zhipu AI".to_string(),
        base_url: "https://open.bigmodel.cn/api/paas/v4".to_string(),
        api_key,
        model,
        extra_headers: None,
    })
}
