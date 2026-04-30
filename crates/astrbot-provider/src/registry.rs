use std::collections::HashMap;

use crate::{ChatProvider, ProviderConfig};

pub struct ProviderRegistry {
    llm_factories: HashMap<String, Box<dyn Fn(ProviderConfig) -> Box<dyn ChatProvider>>>,
}

impl ProviderRegistry {
    pub fn new() -> Self {
        let mut registry = Self {
            llm_factories: HashMap::new(),
        };

        registry.register("openai", |c| {
            Box::new(crate::OpenAiCompatibleProvider::new(c))
        });
        registry.register("moonshot", |c| {
            Box::new(crate::sources::moonshot::create(c.api_key, c.model))
        });
        registry.register("deepseek", |c| {
            Box::new(crate::sources::deepseek::create(c.api_key, c.model))
        });
        registry.register("groq", |c| {
            Box::new(crate::sources::groq::create(c.api_key, c.model))
        });
        registry.register("ollama", |c| {
            Box::new(crate::sources::ollama::OllamaProvider::new(c))
        });
        registry.register("openrouter", |c| {
            Box::new(crate::sources::openrouter::create(c.api_key, c.model))
        });
        registry.register("siliconflow", |c| {
            Box::new(crate::sources::siliconflow::create(c.api_key, c.model))
        });
        registry.register("oneapi", |c| {
            Box::new(crate::sources::oneapi::create(
                c.base_url, c.api_key, c.model,
            ))
        });
        registry.register("lmstudio", |c| {
            Box::new(crate::sources::lmstudio::create(c.base_url, c.model))
        });
        registry.register("zhipu", |c| {
            Box::new(crate::sources::zhipu::create(c.api_key, c.model))
        });
        registry.register("xai", |c| {
            Box::new(crate::sources::xai::create(c.api_key, c.model))
        });
        registry.register("minimax", |c| {
            Box::new(crate::sources::minimax::create(c.api_key, c.model))
        });
        registry.register("volcengine", |c| {
            Box::new(crate::sources::volcengine::create(c.api_key, c.model))
        });
        registry.register("qwen", |c| {
            Box::new(crate::sources::qwen::create(c.api_key, c.model))
        });
        registry.register("stepfun", |c| {
            Box::new(crate::sources::stepfun::create(c.api_key, c.model))
        });
        registry.register("hyperbolic", |c| {
            Box::new(crate::sources::hyperbolic::create(c.api_key, c.model))
        });
        registry.register("ai21", |c| {
            Box::new(crate::sources::ai21::create(c.api_key, c.model))
        });
        registry.register("azure", |c| {
            Box::new(crate::sources::azure::create(c.api_key, c.model))
        });
        registry.register("baichuan", |c| {
            Box::new(crate::sources::baichuan::create(c.api_key, c.model))
        });
        registry.register("claude", |c| {
            Box::new(crate::sources::claude::ClaudeProvider::new(c))
        });
        registry.register("cohere", |c| {
            Box::new(crate::sources::cohere::create(c.api_key, c.model))
        });
        registry.register("fireworks", |c| {
            Box::new(crate::sources::fireworks::create(c.api_key, c.model))
        });
        registry.register("perplexity", |c| {
            Box::new(crate::sources::perplexity::create(c.api_key, c.model))
        });
        registry.register("together", |c| {
            Box::new(crate::sources::together::create(c.api_key, c.model))
        });
        registry.register("zerooneai", |c| {
            Box::new(crate::sources::zerooneai::create(c.api_key, c.model))
        });
        registry.register("baidu", |c| {
            Box::new(crate::sources::baidu::create(c.api_key, c.model))
        });
        registry.register("hunyuan", |c| {
            Box::new(crate::sources::hunyuan::create(c.api_key, c.model))
        });
        registry.register("spark", |c| {
            Box::new(crate::sources::spark::create(c.api_key, c.model))
        });

        registry
    }

    pub fn register(
        &mut self,
        name: &str,
        factory: impl Fn(ProviderConfig) -> Box<dyn ChatProvider> + 'static,
    ) {
        self.llm_factories
            .insert(name.to_string(), Box::new(factory));
    }

    pub fn create(&self, name: &str, config: ProviderConfig) -> Option<Box<dyn ChatProvider>> {
        self.llm_factories.get(name).map(|f| f(config))
    }

    pub fn list(&self) -> Vec<&String> {
        self.llm_factories.keys().collect()
    }
}

impl Default for ProviderRegistry {
    fn default() -> Self {
        Self::new()
    }
}
