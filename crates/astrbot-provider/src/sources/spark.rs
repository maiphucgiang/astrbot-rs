use crate::{OpenAiCompatibleProvider, ProviderConfig};

pub fn create(api_key: String, model: String) -> OpenAiCompatibleProvider {
    OpenAiCompatibleProvider::new(ProviderConfig {
        name: "iFlytek Spark".to_string(),
        base_url: "https://spark-api-open.xf-yun.com/v1".to_string(),
        api_key,
        model,
        extra_headers: None,
    })
}
