//! Provider template system — registry of provider configuration templates
//!
//! Allows quick creation of provider instances from JSON / TOML templates.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A provider template — reusable configuration for creating a provider instance
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderTemplate {
    pub id: String,
    pub name: String,
    pub provider_type: String,
    pub base_url: Option<String>,
    pub default_model: String,
    #[serde(default)]
    pub extra: HashMap<String, serde_json::Value>,
}

/// Template registry — holds all known provider templates
#[derive(Debug, Default)]
pub struct TemplateRegistry {
    templates: HashMap<String, ProviderTemplate>,
}

impl TemplateRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a built-in template
    pub fn register_builtin(&mut self) {
        let builtins = vec![
            ProviderTemplate {
                id: "openai".to_string(),
                name: "OpenAI".to_string(),
                provider_type: "openai".to_string(),
                base_url: Some("https://api.openai.com/v1".to_string()),
                default_model: "gpt-4o".to_string(),
                extra: HashMap::new(),
            },
            ProviderTemplate {
                id: "kimi".to_string(),
                name: "Kimi (Moonshot)".to_string(),
                provider_type: "kimi".to_string(),
                base_url: Some("https://api.moonshot.cn/v1".to_string()),
                default_model: "moonshot-v1-8k".to_string(),
                extra: HashMap::new(),
            },
            ProviderTemplate {
                id: "deepseek".to_string(),
                name: "DeepSeek".to_string(),
                provider_type: "deepseek".to_string(),
                base_url: Some("https://api.deepseek.com/v1".to_string()),
                default_model: "deepseek-chat".to_string(),
                extra: HashMap::new(),
            },
            ProviderTemplate {
                id: "zhipu".to_string(),
                name: "Zhipu AI".to_string(),
                provider_type: "zhipu".to_string(),
                base_url: Some("https://open.bigmodel.cn/api/paas/v4".to_string()),
                default_model: "glm-4".to_string(),
                extra: HashMap::new(),
            },
            ProviderTemplate {
                id: "siliconflow".to_string(),
                name: "SiliconFlow".to_string(),
                provider_type: "siliconflow".to_string(),
                base_url: Some("https://api.siliconflow.cn/v1".to_string()),
                default_model: "deepseek-ai/DeepSeek-V3".to_string(),
                extra: HashMap::new(),
            },
            ProviderTemplate {
                id: "ollama".to_string(),
                name: "Ollama (Local)".to_string(),
                provider_type: "ollama".to_string(),
                base_url: Some("http://localhost:11434".to_string()),
                default_model: "llama3".to_string(),
                extra: HashMap::new(),
            },
            ProviderTemplate {
                id: "baidu".to_string(),
                name: "Baidu Qianfan (ERNIE)".to_string(),
                provider_type: "baidu".to_string(),
                base_url: Some("https://qianfan.baidubce.com/compatible-mode/v1".to_string()),
                default_model: "ernie-4.0-turbo-8k".to_string(),
                extra: HashMap::new(),
            },
            ProviderTemplate {
                id: "qwen".to_string(),
                name: "Qwen (Alibaba)".to_string(),
                provider_type: "qwen".to_string(),
                base_url: Some("https://dashscope.aliyuncs.com/compatible-mode/v1".to_string()),
                default_model: "qwen-turbo".to_string(),
                extra: HashMap::new(),
            },
            ProviderTemplate {
                id: "hunyuan".to_string(),
                name: "Tencent Hunyuan".to_string(),
                provider_type: "hunyuan".to_string(),
                base_url: Some("https://hunyuan.tencentcloudapi.com/compatible-mode/v1".to_string()),
                default_model: "hunyuan-lite".to_string(),
                extra: HashMap::new(),
            },
            ProviderTemplate {
                id: "spark".to_string(),
                name: "iFlytek Spark".to_string(),
                provider_type: "spark".to_string(),
                base_url: Some("https://spark-api-open.xf-yun.com/v1".to_string()),
                default_model: "generalv3.5".to_string(),
                extra: HashMap::new(),
            },
        ];
        for t in builtins {
            self.templates.insert(t.id.clone(), t);
        }
    }

    /// Add a custom template
    pub fn add(&mut self, template: ProviderTemplate) {
        self.templates.insert(template.id.clone(), template);
    }

    /// Get a template by ID
    pub fn get(&self, id: &str) -> Option<&ProviderTemplate> {
        self.templates.get(id)
    }

    /// Remove a template
    pub fn remove(&mut self, id: &str) -> Option<ProviderTemplate> {
        self.templates.remove(id)
    }

    /// List all templates
    pub fn list(&self) -> Vec<&ProviderTemplate> {
        self.templates.values().collect()
    }

    /// Load templates from a JSON string
    pub fn load_json(&mut self, json: &str) -> astrbot_core::errors::Result<()> {
        let templates: Vec<ProviderTemplate> = serde_json::from_str(json)
            .map_err(|e| astrbot_core::errors::AstrBotError::Serialization(format!("Template JSON parse failed: {}", e)))?;
        for t in templates {
            self.templates.insert(t.id.clone(), t);
        }
        Ok(())
    }

    /// Export all templates to a JSON string
    pub fn export_json(&self) -> astrbot_core::errors::Result<String> {
        let templates: Vec<&ProviderTemplate> = self.list();
        serde_json::to_string(&templates)
            .map_err(|e| astrbot_core::errors::AstrBotError::Serialization(format!("Template JSON export failed: {}", e)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_builtin_templates() {
        let mut registry = TemplateRegistry::new();
        registry.register_builtin();
        assert!(registry.get("openai").is_some());
        assert!(registry.get("kimi").is_some());
        assert!(registry.get("deepseek").is_some());
        assert!(registry.get("baidu").is_some());
        assert_eq!(registry.list().len(), 10);
    }

    #[test]
    fn test_template_json_roundtrip() {
        let mut registry = TemplateRegistry::new();
        registry.register_builtin();
        let json = registry.export_json().unwrap();
        let mut registry2 = TemplateRegistry::new();
        registry2.load_json(&json).unwrap();
        assert_eq!(registry2.list().len(), 8);
    }
}
