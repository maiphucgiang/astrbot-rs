use serde::{Deserialize, Serialize};
use std::io::{self, Write};
use std::path::Path;

/// 新用户交互式引导流程
pub struct InteractiveOnboarding {
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

impl InteractiveOnboarding {
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

    /// 运行完整的交互式 onboarding 流程
    pub fn run(&self) -> anyhow::Result<OnboardingConfig> {
        println!("🚀 欢迎使用 AstrBot！让我们花 2 分钟完成初始配置。\n");

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

    /// 第一步：交互式选择主要平台
    fn prompt_platform(&self) -> anyhow::Result<String> {
        let platforms = vec![
            "QQ",
            "Telegram",
            "Discord",
            "飞书",
            "钉钉",
            "微信",
            "Slack",
            "Kook",
            "Line",
            "Satori",
            "公众号",
        ];

        println!("📡 第一步：选择你的主要平台");
        println!("───────────────────────────────");
        for (i, p) in platforms.iter().enumerate() {
            println!("  {}. {}", i + 1, p);
        }
        println!();

        let idx = loop {
            let input = Self::ask("输入编号（1-11）：");
            match input.parse::<usize>() {
                Ok(n) if n >= 1 && n <= platforms.len() => break n - 1,
                _ => println!("❌ 无效输入，请重新选择。"),
            }
        };

        let selected = platforms[idx].to_string();
        println!("✓ 已选择平台: {}\n", selected);
        Ok(selected)
    }

    /// 第二步：交互式选择 Provider + 输入 API Key
    fn prompt_provider(&self) -> anyhow::Result<(String, String)> {
        let providers = vec!["OpenAI", "DeepSeek", "硅基流动", "Gemini", "Ollama", "其他"];

        println!("🤖 第二步：接入 LLM Provider");
        println!("───────────────────────────────");
        for (i, p) in providers.iter().enumerate() {
            println!("  {}. {}", i + 1, p);
        }
        println!();

        let idx = loop {
            let input = Self::ask("输入编号（1-6）：");
            match input.parse::<usize>() {
                Ok(n) if n >= 1 && n <= providers.len() => break n - 1,
                _ => println!("❌ 无效输入，请重新选择。"),
            }
        };

        let provider = providers[idx].to_string();
        println!("✓ 已选择 Provider: {}", provider);

        let api_key = if provider == "Ollama" {
            println!("⚠ Ollama 使用本地模型，无需 API Key");
            String::new()
        } else {
            Self::ask("请输入 API Key（sk-...）：")
        };

        println!();
        Ok((provider, api_key))
    }

    /// 第三步：发送测试请求验证 API Key 连通性
    fn verify_api_key(&self, provider: &str, api_key: &str) -> anyhow::Result<()> {
        if api_key.is_empty() || api_key == "sk-xxx" {
            if provider == "Ollama" {
                println!("✓ Ollama 本地服务，跳过 API Key 验证\n");
                return Ok(());
            }
            anyhow::bail!("API Key 无效，请检查输入");
        }

        println!("🔍 正在验证 API Key 连通性...");

        let (url, auth_header) = match provider {
            "OpenAI" => (
                "https://api.openai.com/v1/models".to_string(),
                Some(format!("Authorization: Bearer {}", api_key)),
            ),
            "DeepSeek" => (
                "https://api.deepseek.com/v1/models".to_string(),
                Some(format!("Authorization: Bearer {}", api_key)),
            ),
            "硅基流动" => (
                "https://api.siliconflow.cn/v1/models".to_string(),
                Some(format!("Authorization: Bearer {}", api_key)),
            ),
            "Gemini" => (
                format!(
                    "https://generativelanguage.googleapis.com/v1beta/models?key={}",
                    api_key
                ),
                None,
            ),
            "Ollama" => ("http://localhost:11434/api/tags".to_string(), None),
            _ => {
                println!("⚠ 未知 Provider，跳过自动验证");
                return Ok(());
            }
        };

        let mut cmd = std::process::Command::new("curl");
        cmd.args(&[
            "-s",
            "-o",
            "/dev/null",
            "-w",
            "%{http_code}",
            "--max-time",
            "10",
        ]);

        if let Some(header) = auth_header {
            cmd.arg("-H").arg(header);
        }

        let output = cmd.arg(&url).output();

        match output {
            Ok(out) if out.status.success() => {
                let code = String::from_utf8_lossy(&out.stdout).trim().to_string();
                match code.as_str() {
                    "200" => {
                        println!("✓ API Key 有效\n");
                        Ok(())
                    }
                    "401" => {
                        anyhow::bail!("API Key 无效或已过期（401 未授权）。请检查 Key 是否正确。")
                    }
                    "429" => anyhow::bail!("请求过于频繁（429），Provider 限流中。请稍后重试。"),
                    _ => anyhow::bail!(
                        "验证失败，HTTP 状态码: {}。可能是网络问题或 Key 无效。",
                        code
                    ),
                }
            }
            _ => {
                anyhow::bail!(
                    "无法连接到 Provider。请检查：1）网络连接 2）API Key 是否正确 3）Provider 服务是否正常"
                )
            }
        }
    }

    /// 第四步：设置管理员（可选）
    fn prompt_admin(&self) -> anyhow::Result<Option<String>> {
        println!("👤 第四步：设置管理员（可选）");
        println!("───────────────────────────────");
        println!("管理员可以执行敏感操作，如重启 Bot、修改配置等。");
        println!();

        if !Self::ask_yes_no("是否现在设置管理员？", false) {
            println!("ℹ 跳过。你可以在运行后通过日志查看你的平台 UID，再手动配置。");
            println!("   提示：发送任意消息后，查看日志中的 sender UID。\n");
            return Ok(None);
        }

        let uid = Self::ask("请输入你的平台 UID（数字 ID）：");
        if uid.is_empty() {
            println!("ℹ 未输入 UID，跳过。\n");
            return Ok(None);
        }

        println!("✓ 管理员已设置: {}\n", uid);
        Ok(Some(uid))
    }

    /// 第五步：知识库（可选）
    fn prompt_knowledge_base(&self) -> anyhow::Result<bool> {
        println!("📚 第五步：知识库（可选）");
        println!("───────────────────────────────");
        println!("知识库让 Bot 能基于你上传的文档回答问题。");
        println!();

        if !Self::ask_yes_no("是否启用知识库？", false) {
            println!("✗ 未启用知识库\n");
            return Ok(false);
        }

        println!("✓ 已启用知识库");
        println!("📂 请将你的文档放入以下目录：");
        println!("   {}/data/knowledge/", std::env::current_dir()?.display());
        println!("   支持的格式：.txt, .md, .pdf, .docx\n");
        Ok(true)
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
        println!("───────────────────────────────");
        println!("   平台: {}", config.platform);
        println!("   模型: {}", config.provider);
        if let Some(ref admin) = config.admin_id {
            println!("   管理员: {}", admin);
        }
        if config.enable_knowledge_base {
            println!("   知识库: 已启用");
        }
        println!();
        println!("   发送 /help 查看所有可用指令");
        println!("   使用 astrbot logs -f 实时查看日志");
        println!("   文档: https://astrbot.app");
        println!();
    }

    /// 向用户提问，读取一行输入
    fn ask(prompt: &str) -> String {
        print!("{}", prompt);
        let _ = io::stdout().flush();
        let mut input = String::new();
        io::stdin().read_line(&mut input).ok();
        input.trim().to_string()
    }

    /// 询问是/否，返回布尔值
    fn ask_yes_no(prompt: &str, default: bool) -> bool {
        let hint = if default { "(Y/n)" } else { "(y/N)" };
        let input = Self::ask(&format!("{} {} ", prompt, hint));
        match input.to_lowercase().as_str() {
            "y" | "yes" | "true" | "1" => true,
            "n" | "no" | "false" | "0" => false,
            "" => default,
            _ => default,
        }
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
        let flow = InteractiveOnboarding::with_config_path("/nonexistent/config.yaml".to_string());
        assert!(flow.should_trigger());
    }

    #[test]
    fn test_should_not_trigger_when_config_exists() {
        let path = tmp_path("config-exists");
        std::fs::write(&path, "test: true").unwrap();
        let flow = InteractiveOnboarding::with_config_path(path.clone());
        assert!(!flow.should_trigger());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_skip_env_var() {
        std::env::set_var("ASTRBOT_SKIP_ONBOARDING", "1");
        let flow = InteractiveOnboarding::new();
        // 即使配置不存在，设置了环境变量也不触发
        assert!(!flow.should_trigger());
        std::env::remove_var("ASTRBOT_SKIP_ONBOARDING");
    }

    #[test]
    fn test_verify_api_key_empty_fails() {
        let flow = InteractiveOnboarding::new();
        let result = flow.verify_api_key("OpenAI", "");
        assert!(result.is_err());
    }

    #[test]
    fn test_verify_api_key_placeholder_fails() {
        let flow = InteractiveOnboarding::new();
        let result = flow.verify_api_key("OpenAI", "sk-xxx");
        assert!(result.is_err());
    }

    #[test]
    fn test_verify_api_key_ollama_skips() {
        let flow = InteractiveOnboarding::new();
        let result = flow.verify_api_key("Ollama", "");
        assert!(result.is_ok());
    }

    #[test]
    fn test_save_and_load_config() {
        let path = tmp_path("save-load");
        let flow = InteractiveOnboarding::with_config_path(path.clone());

        let config = OnboardingConfig {
            platform: "Telegram".to_string(),
            provider: "DeepSeek".to_string(),
            api_key: "sk-test".to_string(),
            admin_id: Some("123456".to_string()),
            enable_knowledge_base: true,
        };

        flow.save_config(&config).unwrap();
        assert!(Path::new(&path).exists());

        let loaded: OnboardingConfig =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(loaded.platform, "Telegram");
        assert_eq!(loaded.provider, "DeepSeek");

        let _ = std::fs::remove_file(&path);
    }
}

/// 兼容性别名，保留旧名称引用
pub type OnboardingFlow = InteractiveOnboarding;
