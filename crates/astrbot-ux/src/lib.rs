pub mod errors;
pub mod help;
pub mod onboarding;

use std::sync::Arc;

/// UX 模块入口，持有各子系统实例
pub struct UxModule {
    pub onboarding: Arc<onboarding::OnboardingFlow>,
    pub help: Arc<help::HelpSystem>,
    pub errors: Arc<errors::ErrorTranslator>,
}

impl UxModule {
    pub fn new() -> Self {
        Self {
            onboarding: Arc::new(onboarding::OnboardingFlow::new()),
            help: Arc::new(help::HelpSystem::new()),
            errors: Arc::new(errors::ErrorTranslator::new()),
        }
    }

    /// 检查是否需要启动 onboarding 流程
    pub fn should_onboard(&self) -> bool {
        self.onboarding.should_trigger()
    }

    /// 运行 onboarding
    pub fn run_onboarding(&self) -> anyhow::Result<onboarding::OnboardingConfig> {
        self.onboarding.run()
    }

    /// 获取帮助文本
    pub fn get_help(&self, scope: help::HelpScope) -> String {
        self.help.render(scope)
    }

    /// 翻译技术错误为用户友好消息
    pub fn translate_error(&self, raw: &str) -> errors::HumanizedError {
        self.errors.translate(raw)
    }
}
