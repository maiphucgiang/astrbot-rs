use async_trait::async_trait;
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::errors::{AstrBotError, Result};

/// Trait for text-to-image generators
#[async_trait]
pub trait ImageGenerator: Send + Sync {
    /// Get generator name
    fn name(&self) -> &str;
    /// Generate an image from a text prompt
    async fn generate(&self, prompt: &str, options: ImageOptions) -> Result<ImageResult>;
    /// Check if the generator is healthy / reachable
    async fn health_check(&self) -> Result<bool>;
}

/// Options for image generation
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageOptions {
    /// Image width in pixels
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    /// Image height in pixels
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
    /// Model identifier to use
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Style hint (e.g. "vivid", "natural")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub style: Option<String>,
    /// Number of images to generate
    #[serde(skip_serializing_if = "Option::is_none")]
    pub n: Option<u32>,
    /// Extra provider-specific parameters
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

impl ImageOptions {
    /// Create default options
    pub fn new() -> Self {
        Self::default()
    }

    /// Set width
    pub fn with_width(mut self, width: u32) -> Self {
        self.width = Some(width);
        self
    }

    /// Set height
    pub fn with_height(mut self, height: u32) -> Self {
        self.height = Some(height);
        self
    }

    /// Set model
    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = Some(model.into());
        self
    }

    /// Set style
    pub fn with_style(mut self, style: impl Into<String>) -> Self {
        self.style = Some(style.into());
        self
    }

    /// Set number of images
    pub fn with_n(mut self, n: u32) -> Self {
        self.n = Some(n);
        self
    }
}

/// Result of an image generation request
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ImageResult {
    /// Public URL of the generated image (if provided by backend)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// Base64-encoded image data (if provided by backend)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base64: Option<String>,
    /// Revised / expanded prompt (e.g. DALL-E returns this)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_revised: Option<String>,
}

/// Registry that holds multiple image-generator backends
pub struct T2IRegistry {
    generators: DashMap<String, Box<dyn ImageGenerator>>,
}

impl Default for T2IRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl T2IRegistry {
    /// Create an empty registry
    pub fn new() -> Self {
        Self {
            generators: DashMap::new(),
        }
    }

    /// Register a generator backend
    pub fn register(&self, generator: Box<dyn ImageGenerator>) {
        let name = generator.name().to_string();
        self.generators.insert(name, generator);
    }

    /// Get a generator by name
    pub fn get(&self, name: &str) -> Option<dashmap::mapref::one::Ref<'_, String, Box<dyn ImageGenerator>>> {
        self.generators.get(name)
    }

    /// List all registered generator names
    pub fn list(&self) -> Vec<String> {
        self.generators.iter().map(|e| e.key().clone()).collect()
    }

    /// Generate using a specific backend by name
    pub async fn generate_with(
        &self,
        name: &str,
        prompt: &str,
        options: ImageOptions,
    ) -> Result<ImageResult> {
        let generator = self
            .generators
            .get(name)
            .ok_or_else(|| AstrBotError::NotFound(format!("generator '{}' not found", name)))?;
        generator.generate(prompt, options).await
    }

    /// Health-check a specific backend by name
    pub async fn health_check(&self, name: &str) -> Result<bool> {
        let generator = self
            .generators
            .get(name)
            .ok_or_else(|| AstrBotError::NotFound(format!("generator '{}' not found", name)))?;
        generator.health_check().await
    }
}

/// DALL-E 3 image generator via OpenAI API
pub struct DallEGenerator {
    api_key: String,
    base_url: String,
    client: reqwest::Client,
}

impl DallEGenerator {
    /// Create a new DALL-E generator
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            base_url: "https://api.openai.com".to_string(),
            client: reqwest::Client::new(),
        }
    }

    /// Create with custom base URL (e.g. proxy)
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    /// Create with custom HTTP client
    pub fn with_client(mut self, client: reqwest::Client) -> Self {
        self.client = client;
        self
    }
}

#[async_trait]
impl ImageGenerator for DallEGenerator {
    fn name(&self) -> &str {
        "dall-e"
    }

    async fn generate(&self, prompt: &str, options: ImageOptions) -> Result<ImageResult> {
        let size = match (options.width, options.height) {
            (Some(1024), Some(1792)) | (Some(1792), Some(1024)) => {
                if options.width == Some(1024) {
                    "1024x1792"
                } else {
                    "1792x1024"
                }
            }
            (Some(512), Some(512)) => "512x512",
            (Some(256), Some(256)) => "256x256",
            _ => "1024x1024",
        };

        let body = serde_json::json!({
            "model": options.model.as_deref().unwrap_or("dall-e-3"),
            "prompt": prompt,
            "n": options.n.unwrap_or(1).min(1), // DALL-E 3 only supports n=1
            "size": size,
        });

        let url = format!("{}/v1/images/generations", self.base_url.trim_end_matches('/'));
        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&body)
            .send()
            .await
            .map_err(|e| AstrBotError::Network(format!("DALL-E request failed: {}", e)))?;

        if !response.status().is_success() {
            let text = response
                .text()
                .await
                .unwrap_or_default();
            return Err(AstrBotError::Provider {
                provider: "dall-e".to_string(),
                message: format!("OpenAI API error: {}", text),
            });
        }

        let json: serde_json::Value = response
            .json()
            .await
            .map_err(|e| AstrBotError::Serialization(format!("DALL-E response parse: {}", e)))?;

        let image_url = json
            .get("data")
            .and_then(|d| d.as_array())
            .and_then(|arr| arr.first())
            .and_then(|item| item.get("url"))
            .and_then(|u| u.as_str())
            .map(|s| s.to_string());

        let revised_prompt = json
            .get("data")
            .and_then(|d| d.as_array())
            .and_then(|arr| arr.first())
            .and_then(|item| item.get("revised_prompt"))
            .and_then(|r| r.as_str())
            .map(|s| s.to_string());

        Ok(ImageResult {
            url: image_url,
            base64: None,
            prompt_revised: revised_prompt,
        })
    }

    async fn health_check(&self) -> Result<bool> {
        // Check by hitting the models endpoint (lightweight)
        let url = format!("{}/v1/models", self.base_url.trim_end_matches('/'));
        let result = self
            .client
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .send()
            .await;
        match result {
            Ok(resp) => Ok(resp.status().is_success()),
            Err(_) => Ok(false),
        }
    }
}

/// Stable Diffusion WebUI (AUTOMATIC1111) txt2img generator
pub struct StableDiffusionGenerator {
    host: String,
    client: reqwest::Client,
    default_steps: u32,
    default_width: u32,
    default_height: u32,
}

impl StableDiffusionGenerator {
    /// Create a new SD generator pointing at a WebUI host
    pub fn new(host: impl Into<String>) -> Self {
        Self {
            host: host.into(),
            client: reqwest::Client::new(),
            default_steps: 20,
            default_width: 512,
            default_height: 512,
        }
    }

    /// Create with custom HTTP client
    pub fn with_client(mut self, client: reqwest::Client) -> Self {
        self.client = client;
        self
    }

    /// Set default generation parameters
    pub fn with_defaults(mut self, steps: u32, width: u32, height: u32) -> Self {
        self.default_steps = steps;
        self.default_width = width;
        self.default_height = height;
        self
    }
}

#[async_trait]
impl ImageGenerator for StableDiffusionGenerator {
    fn name(&self) -> &str {
        "stable-diffusion"
    }

    async fn generate(&self, prompt: &str, options: ImageOptions) -> Result<ImageResult> {
        let body = serde_json::json!({
            "prompt": prompt,
            "steps": self.default_steps,
            "width": options.width.unwrap_or(self.default_width),
            "height": options.height.unwrap_or(self.default_height),
        });

        let url = format!("{}/sdapi/v1/txt2img", self.host.trim_end_matches('/'));
        let response = self
            .client
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| AstrBotError::Network(format!("SD request failed: {}", e)))?;

        if !response.status().is_success() {
            let text = response
                .text()
                .await
                .unwrap_or_default();
            return Err(AstrBotError::Provider {
                provider: "stable-diffusion".to_string(),
                message: format!("SD API error: {}", text),
            });
        }

        let json: serde_json::Value = response
            .json()
            .await
            .map_err(|e| AstrBotError::Serialization(format!("SD response parse: {}", e)))?;

        let base64_image = json
            .get("images")
            .and_then(|i| i.as_array())
            .and_then(|arr| arr.first())
            .and_then(|item| item.as_str())
            .map(|s| s.to_string());

        Ok(ImageResult {
            url: None,
            base64: base64_image,
            prompt_revised: None,
        })
    }

    async fn health_check(&self) -> Result<bool> {
        let url = format!("{}/sdapi/v1/samplers", self.host.trim_end_matches('/'));
        let result = self.client.get(&url).send().await;
        match result {
            Ok(resp) => Ok(resp.status().is_success()),
            Err(_) => Ok(false),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_image_options_default() {
        let opts = ImageOptions::default();
        assert!(opts.width.is_none());
        assert!(opts.height.is_none());
        assert!(opts.model.is_none());
        assert!(opts.style.is_none());
        assert!(opts.n.is_none());
        assert!(opts.extra.is_empty());
    }

    #[test]
    fn test_image_options_builder() {
        let opts = ImageOptions::new()
            .with_width(1024)
            .with_height(768)
            .with_model("dall-e-3")
            .with_style("vivid")
            .with_n(2);

        assert_eq!(opts.width, Some(1024));
        assert_eq!(opts.height, Some(768));
        assert_eq!(opts.model, Some("dall-e-3".to_string()));
        assert_eq!(opts.style, Some("vivid".to_string()));
        assert_eq!(opts.n, Some(2));
    }

    #[tokio::test]
    async fn test_t2i_registry_register_get_list() {
        let registry = T2IRegistry::new();

        let dalle = Box::new(DallEGenerator::new("sk-test"));
        registry.register(dalle);

        let sd = Box::new(StableDiffusionGenerator::new("http://localhost:7860"));
        registry.register(sd);

        let names = registry.list();
        assert_eq!(names.len(), 2);
        assert!(names.contains(&"dall-e".to_string()));
        assert!(names.contains(&"stable-diffusion".to_string()));

        // Verify get works
        assert!(registry.get("dall-e").is_some());
        assert!(registry.get("stable-diffusion").is_some());
        assert!(registry.get("nonexistent").is_none());

        // generate_with should delegate (but will fail network-wise)
        let result = registry
            .generate_with("dall-e", "a cat", ImageOptions::default())
            .await;
        // We expect an error because the API key is fake
        assert!(result.is_err());
    }

    #[test]
    fn test_dalle_generator_creation() {
        let generator = DallEGenerator::new("sk-abc123");
        assert_eq!(generator.name(), "dall-e");

        let generator2 = DallEGenerator::new("sk-xyz")
            .with_base_url("https://proxy.example.com");
        assert_eq!(generator2.name(), "dall-e");
    }

    #[test]
    fn test_sd_generator_creation() {
        let generator = StableDiffusionGenerator::new("http://127.0.0.1:7860");
        assert_eq!(generator.name(), "stable-diffusion");

        let generator2 = StableDiffusionGenerator::new("http://localhost:7860")
            .with_defaults(30, 768, 768);
        assert_eq!(generator2.name(), "stable-diffusion");
    }
}
