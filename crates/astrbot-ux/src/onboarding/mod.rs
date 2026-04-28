use std::path::Path;
use serde::{Deserialize, Serialize};

/// 新用户引导流程
pub struct OnboardingFlow {
    config_path: String,
}

/// Onboarding 配置结果
#[derive(Debug, Serialize, Deserialize)]
pub struct OnboardingConfig {
    /// 选择的主要平台
    pub platform: String,
    /// 选择的 Provider
    pub provider: String,
    /// API Key（加密存储）
    pub api_key: String,
    /// 管理员 ID
    pub admin_id: Option<String>,
    /// 是否启用知识库
    pub enable_knowledge_base: bool,
}

/// Onboarding 每一步的状态
#[derive(Debug, Clone, PartialEq)]
pub enum OnboardingStep {
    /// 检测配置
    CheckConfig,
    /// 第一步：选择平台
    SelectPlatform,
    /// 第二步：接入 LLM Provider
    SetupProvider,
    /// 第三步：验证 API Key
    VerifyApiKey,
    /// 第四步：设置管理员
    SetAdmin,
    /// 第五步：知识库（可选）
    KnowledgeBase,
    /// 完成
    Complete,
}

impl OnboardingFlow {
    pub fn new() -> Self {
        Self {
            config_path: "data/config.yaml".to_string(),
        }
    }

    pub fn with_config_path(path: String) -> Self {
        Self { config_path: path }
    }

    /// 检测是否需要触发 onboarding
    pub fn should_trigger(&self) -> bool {
        if Self::skip_requested() {
            return false;
        }
        !Path::new(&self.config_path).exists()
    }

    /// 检查是否用户要求跳过
    pub fn skip_requested() -> bool {
        std::env::var("ASTRBOT_SKIP_ONBOARDING").is_ok()
            || std::env::args().any(|a| a == "--skip-onboarding")
    }

    /// 运行完整的 onboarding 流程
    pub fn run(&self) -> anyhow::Result<OnboardingConfig> {
        let mut config = OnboardingConfig {
            platform: String::new(),
            provider: String::new(),
            api_key: String::new(),
            admin_id: None,
            enable_knowledge_base: false,
        };

        // Step 1: 选择平台
        config.platform = self.prompt_platform()?;

        // Step 2: 接入 Provider
        let (provider, api_key) = self.prompt_provider()?;
        config.provider = provider;
        config.api_key = api_key;

        // Step 3: 验证 API Key
        self.verify_api_key(&config.provider, &config.api_key)?;

        // Step 4: 设置管理员（可选）
        config.admin_id = self.prompt_admin()?;

        // Step 5: 知识库（可选）
        config.enable_knowledge_base = self.prompt_knowledge_base()?;

        // 保存配置
        self.save_config(&config)?;

        // 显示完成消息
        self.show_completion(&config);

        Ok(config)
    }

    /// 第一步：选择主要平台
    fn prompt_platform(&self) -> anyhow::Result<String> {
        // TODO: 交互式选择
        // 平台列表：QQ, Telegram, Discord, 飞书, 钉钉, 微信, Slack, Kook, Line, Satori, 公众号
        Ok("QQ".to_string())
    }

    /// 第二步：接入 LLM Provider
    fn prompt_provider(&self) -> anyhow::Result<(String, String)> {
        // TODO: 交互式选择 + API Key 输入
        // Provider: OpenAI, DeepSeek, 硅基流动, Gemini, Ollama, 其他
        Ok(("OpenAI".to_string(), "sk-xxx".to_string()))
    }

    /// 第三步：验证 API Key 有效性
    fn verify_api_key(&self, _provider: &str, api_key: &str) -> anyhow::Result<()> {
        // TODO: 发送一次测试请求验证连通性
        // 成功：显示 "✓ API Key 有效"
        // 失败：给出具体原因（网络/Key错误/余额不足）
        if api_key.is_empty() || api_key == "sk-xxx" {
            anyhow::bail!("API Key 无效，请检查输入");
        }
        Ok(())
    }

    /// 第四步：设置管理员（可选）
    fn prompt_admin(&self) -> anyhow::Result<Option<String>> {
        // TODO: 询问管理员平台 UID
        // 提供 "我不知道" 选项，引导查看日志
        Ok(None)
    }

    /// 第五步：知识库（可选）
    fn prompt_knowledge_base(&self) -> anyhow::Result<bool> {
        // TODO: 询问是否启用知识库
        // 是：提示放入文件到 data/knowledge/
        Ok(false)
    }

    /// 保存配置到文件
    fn save_config(&self, config: &OnboardingConfig) -> anyhow::Result<()> {
        let json = serde_json::to_string_pretty(config)?;
        std::fs::write(&self.config_path, json)?;
        Ok(())
    }

    /// 显示完成消息
    fn show_completion(&self, config: &OnboardingConfig) {
        println!("🎉 你的 AstrBot 已就绪！");
        println!("   平台: {}", config.platform);
        println!("   模型: {}", config.provider);
        println!("   发送 /help 查看所有可用指令");
        println!("   使用 astrbot logs -f 实时查看日志");
        println!("   文档: https://astrbot.app");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_path(name: &str) -> String {
        let path = format!("/tmp/astrbot-test-{}", name);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir_all(&path);
        path
    }

    #[test]
    fn test_should_trigger_when_config_missing() {
        let flow = OnboardingFlow::with_config_path("/nonexistent/config.yaml".to_string());
        assert!(flow.should_trigger());
    }

    #[test]
    fn test_should_not_trigger_when_config_exists() {
        let path = tmp_path("config-exists");
        std::fs::write(&path, "test: true").unwrap();
        let flow = OnboardingFlow::with_config_path(path.clone());
        assert!(!flow.should_trigger());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_skip_env_var() {
        std::env::set_var("ASTRBOT_SKIP_ONBOARDING", "1");
        let flow = OnboardingFlow::new();
        // 即使配置不存在，设置了环境变量也不触发
        assert!(!flow.should_trigger());
        std::env::remove_var("ASTRBOT_SKIP_ONBOARDING");
    }

    #[test]
    fn test_verify_api_key_empty_fails() {
        let flow = OnboardingFlow::new();
        let result = flow.verify_api_key("OpenAI", "");
        assert!(result.is_err());
    }

    #[test]
    fn test_save_and_load_config() {
        let path = tmp_path("save-load");
        let flow = OnboardingFlow::with_config_path(path.clone());

        let config = OnboardingConfig {
            platform: "Telegram".to_string(),
            provider: "DeepSeek".to_string(),
            api_key: "sk-test".to_string(),
            admin_id: Some("123456".to_string()),
            enable_knowledge_base: true,
        };

        flow.save_config(&config).unwrap();
        assert!(Path::new(&path).exists());

        let loaded: OnboardingConfig = serde_json::from_str(
            &std::fs::read_to_string(&path).unwrap()
        ).unwrap();
        assert_eq!(loaded.platform, "Telegram");
        assert_eq!(loaded.provider, "DeepSeek");

        let _ = std::fs::remove_file(&path);
    }
}
