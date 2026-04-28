use std::collections::HashMap;
use lazy_static::lazy_static;

/// 错误翻译器：将技术错误转换为用户友好的消息
pub struct ErrorTranslator;

/// 错误代码
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ErrorCode {
    // Provider 类
    ProviderRateLimit,
    ProviderInvalidKey,
    ProviderTimeout,
    ProviderModelNotFound,
    ProviderContextExceeded,
    ProviderBalanceEmpty,
    
    // 平台适配器类
    PlatformConnectionFailed,
    PlatformAuthFailed,
    PlatformMessageTooLong,
    PlatformBanned,
    
    // 数据库类
    DbUniqueConstraint,
    DbNotFound,
    DbConnectionFailed,
    
    // 配置类
    ConfigMissing,
    ConfigInvalid,
    
    // 系统类
    PluginNotFound,
    PluginLoadFailed,
    PermissionDenied,
    
    // 通用
    NetworkError,
    Unknown,
}

/// 错误级别，决定语气和建议
#[derive(Debug, Clone, PartialEq)]
pub enum ErrorLevel {
    /// 致命错误，启动失败
    Fatal,
    /// 运行时错误，功能不可用
    Error,
    /// 警告，功能降级但仍可用
    Warning,
    /// 提示，用户操作不当
    Hint,
}

/// 翻译后的错误消息
#[derive(Debug, Clone)]
pub struct HumanizedError {
    pub code: ErrorCode,
    pub level: ErrorLevel,
    /// 用户看到的标题
    pub title: String,
    /// 具体原因
    pub reason: String,
    /// 建议操作
    pub suggestion: String,
}

lazy_static! {
    /// ErrorCode → 翻译模板
    static ref ERROR_TEMPLATES: HashMap<ErrorCode, (&'static str, &'static str, &'static str)> = {
        let mut m = HashMap::new();
        
        // Provider 类
        m.insert(ErrorCode::ProviderRateLimit, (
            "请求太快了",
            "服务商限制了请求频率",
            "等一分钟再试，或者换个模型。"
        ));
        m.insert(ErrorCode::ProviderInvalidKey, (
            "API Key 有问题",
            "密钥无效或已过期",
            "检查 config 里的 api_key 有没有填对，或者去服务商后台重新生成一个。"
        ));
        m.insert(ErrorCode::ProviderTimeout, (
            "模型响应超时",
            "服务商响应太慢或者网络不稳定",
            "再试一次，如果经常超时可以考虑换个响应更快的模型。"
        ));
        m.insert(ErrorCode::ProviderModelNotFound, (
            "找不到这个模型",
            "模型名称可能打错了，或者服务商已下架",
            "用 /model 看看有哪些可用的模型。"
        ));
        m.insert(ErrorCode::ProviderContextExceeded, (
            "对话太长了",
            "超过了模型的上下文长度限制",
            "用 /reset 重置一下会话记忆，或者开启自动总结。"
        ));
        m.insert(ErrorCode::ProviderBalanceEmpty, (
            "余额不足",
            "服务商账户里没有余额了",
            "去服务商后台充值，或者切换到免费的模型（如 Ollama 本地部署）。"
        ));
        
        // 平台适配器类
        m.insert(ErrorCode::PlatformConnectionFailed, (
            "暂时连不上平台",
            "网络问题或平台服务维护",
            "正在自动重试...如果持续失败，检查一下网络连接和平台状态。"
        ));
        m.insert(ErrorCode::PlatformAuthFailed, (
            "平台认证失败",
            "Token 过期或权限不足",
            "检查一下平台配置里的 token 是否有效，可能需要重新获取。"
        ));
        m.insert(ErrorCode::PlatformMessageTooLong, (
            "消息太长了",
            "超过了平台单条消息长度限制",
            "把消息拆成几段发送，或者开启自动分段。"
        ));
        m.insert(ErrorCode::PlatformBanned, (
            "发送被限制了",
            "平台风控或触发敏感词",
            "检查一下消息内容，避免敏感词。等几分钟再试。"
        ));
        
        // 数据库类
        m.insert(ErrorCode::DbUniqueConstraint, (
            "这条记录已经存在了",
            "数据库里已经有相同的数据",
            "如果你是想更新它，用修改功能而不是新建。"
        ));
        m.insert(ErrorCode::DbNotFound, (
            "找不到这条记录",
            "数据库里没有你要找的数据",
            "检查一下 ID 或名称是否输入正确。"
        ));
        m.insert(ErrorCode::DbConnectionFailed, (
            "数据库连不上",
            "SQLite 文件可能被占用或损坏",
            "检查一下 data/ 目录权限，或者重启一下服务。"
        ));
        
        // 配置类
        m.insert(ErrorCode::ConfigMissing, (
            "缺少配置文件",
            "首次启动或配置文件被删除",
            "运行 onboarding 流程自动创建，或者手动复制一份 config.example.yaml。"
        ));
        m.insert(ErrorCode::ConfigInvalid, (
            "配置文件格式不对",
            "YAML 语法错误或缺少必填项",
            "用 YAML 校验工具检查一下，或者对照文档逐项排查。"
        ));
        
        // 系统类
        m.insert(ErrorCode::PluginNotFound, (
            "插件没找到",
            "名字可能打错了，或者还没安装",
            "用 /plugin list 看看有哪些可用的，或者 /plugin install 安装新插件。"
        ));
        m.insert(ErrorCode::PluginLoadFailed, (
            "插件加载失败",
            "插件依赖缺失或版本不兼容",
            "检查一下插件文档的依赖要求，或者尝试更新插件版本。"
        ));
        m.insert(ErrorCode::PermissionDenied, (
            "权限不够",
            "这个操作需要管理员权限",
            "如果你就是管理员，把 QQ 号/用户 ID 填进 config 的 admin_id 里。"
        ));
        
        // 通用
        m.insert(ErrorCode::NetworkError, (
            "网络出问题了",
            "可能是 DNS、防火墙或代理配置",
            "检查一下网络连接，如果有代理检查一下代理设置。"
        ));
        m.insert(ErrorCode::Unknown, (
            "出了点问题",
            "未知错误",
            "把错误日志发给开发者看看，或者重启一下试试。"
        ));
        
        m
    };
}

impl ErrorTranslator {
    pub fn new() -> Self {
        Self
    }

    /// 将技术错误消息翻译为人类可读消息
    pub fn translate(&self, raw_error: &str) -> HumanizedError {
        let code = self.detect_code(raw_error);
        let level = self.infer_level(&code);
        
        let (title, reason, suggestion) = ERROR_TEMPLATES
            .get(&code)
            .copied()
            .unwrap_or(ERROR_TEMPLATES.get(&ErrorCode::Unknown).copied().unwrap());
        
        HumanizedError {
            code: code.clone(),
            level,
            title: title.to_string(),
            reason: reason.to_string(),
            suggestion: suggestion.to_string(),
        }
    }

    /// 检测错误代码
    fn detect_code(&self, raw: &str) -> ErrorCode {
        let raw_lower = raw.to_lowercase();
        
        // Provider 类
        if raw_lower.contains("429") || raw_lower.contains("rate limit") || raw_lower.contains("too many requests") {
            return ErrorCode::ProviderRateLimit;
        }
        if raw_lower.contains("invalid api key") || raw_lower.contains("unauthorized") || raw_lower.contains("api key") {
            return ErrorCode::ProviderInvalidKey;
        }
        if raw_lower.contains("timeout") && raw_lower.contains("provider") {
            return ErrorCode::ProviderTimeout;
        }
        if raw_lower.contains("model not found") || raw_lower.contains("model does not exist") {
            return ErrorCode::ProviderModelNotFound;
        }
        if raw_lower.contains("context length") || raw_lower.contains("context exceeded") || raw_lower.contains("maximum context") {
            return ErrorCode::ProviderContextExceeded;
        }
        if raw_lower.contains("insufficient balance") || raw_lower.contains("quota exceeded") || raw_lower.contains("no credit") {
            return ErrorCode::ProviderBalanceEmpty;
        }
        
        // 平台适配器类
        if raw_lower.contains("platformadapter connection failed") || raw_lower.contains("platform connection") {
            return ErrorCode::PlatformConnectionFailed;
        }
        if raw_lower.contains("platform auth") || raw_lower.contains("platform token") {
            return ErrorCode::PlatformAuthFailed;
        }
        if raw_lower.contains("message too long") || raw_lower.contains("exceeds max length") {
            return ErrorCode::PlatformMessageTooLong;
        }
        if raw_lower.contains("banned") || raw_lower.contains("blocked") || raw_lower.contains("restricted") {
            return ErrorCode::PlatformBanned;
        }
        
        // 数据库类
        if raw_lower.contains("unique constraint") || raw_lower.contains("duplicate entry") {
            return ErrorCode::DbUniqueConstraint;
        }
        if raw_lower.contains("sqlx error") && raw_lower.contains("not found") {
            return ErrorCode::DbNotFound;
        }
        if raw_lower.contains("sqlx error") && raw_lower.contains("connection") {
            return ErrorCode::DbConnectionFailed;
        }
        
        // 配置类
        if raw_lower.contains("config file not found") || raw_lower.contains("missing config") {
            return ErrorCode::ConfigMissing;
        }
        if raw_lower.contains("config") && raw_lower.contains("invalid") {
            return ErrorCode::ConfigInvalid;
        }
        
        // 系统类
        if raw_lower.contains("plugin not found") {
            return ErrorCode::PluginNotFound;
        }
        if raw_lower.contains("plugin") && raw_lower.contains("load") && raw_lower.contains("fail") {
            return ErrorCode::PluginLoadFailed;
        }
        if raw_lower.contains("permission denied") || raw_lower.contains("access denied") || raw_lower.contains("forbidden") {
            return ErrorCode::PermissionDenied;
        }
        
        // 通用
        if raw_lower.contains("network") || raw_lower.contains("dns") || raw_lower.contains("connection refused") {
            return ErrorCode::NetworkError;
        }
        
        ErrorCode::Unknown
    }

    /// 推断错误级别
    fn infer_level(&self, code: &ErrorCode) -> ErrorLevel {
        match code {
            ErrorCode::ConfigMissing | ErrorCode::ConfigInvalid => ErrorLevel::Fatal,
            ErrorCode::ProviderRateLimit | ErrorCode::PlatformConnectionFailed | ErrorCode::PlatformBanned => ErrorLevel::Warning,
            ErrorCode::PermissionDenied | ErrorCode::PlatformMessageTooLong | ErrorCode::DbUniqueConstraint => ErrorLevel::Hint,
            ErrorCode::PluginNotFound | ErrorCode::DbNotFound => ErrorLevel::Hint,
            _ => ErrorLevel::Error,
        }
    }

    /// 格式化为聊天场景消息
    pub fn format_chat(&self, humanized: &HumanizedError) -> String {
        match humanized.level {
            ErrorLevel::Fatal => format!(
                "💥 {}\n   {}\n   👉 {}",
                humanized.title, humanized.reason, humanized.suggestion
            ),
            ErrorLevel::Error => format!(
                "💢 {}\n   {}\n   👉 {}",
                humanized.title, humanized.reason, humanized.suggestion
            ),
            ErrorLevel::Warning => format!(
                "⚠️ {}\n   {}\n   👉 {}",
                humanized.title, humanized.reason, humanized.suggestion
            ),
            ErrorLevel::Hint => format!(
                "💡 {}\n   {}",
                humanized.title, humanized.suggestion
            ),
        }
    }

    /// 格式化为 CLI 启动错误
    pub fn format_cli(&self, humanized: &HumanizedError) -> String {
        format!(
            "✗ {}: {}\n  → {}",
            humanized.title, humanized.reason, humanized.suggestion
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_translate_rate_limit() {
        let translator = ErrorTranslator::new();
        let err = translator.translate("Provider API returned 429");
        assert_eq!(err.code, ErrorCode::ProviderRateLimit);
        assert_eq!(err.title, "请求太快了");
        assert!(err.suggestion.contains("等一分钟"));
    }

    #[test]
    fn test_translate_connection_failed() {
        let translator = ErrorTranslator::new();
        let err = translator.translate("PlatformAdapter connection failed: qq");
        assert_eq!(err.code, ErrorCode::PlatformConnectionFailed);
        assert_eq!(err.title, "暂时连不上平台");
    }

    #[test]
    fn test_translate_unique_constraint() {
        let translator = ErrorTranslator::new();
        let err = translator.translate("SQLx error: UNIQUE constraint failed: plugins.name");
        assert_eq!(err.code, ErrorCode::DbUniqueConstraint);
        assert_eq!(err.title, "这条记录已经存在了");
    }

    #[test]
    fn test_translate_invalid_key() {
        let translator = ErrorTranslator::new();
        let err = translator.translate("Invalid API key: sk-xxx");
        assert_eq!(err.code, ErrorCode::ProviderInvalidKey);
        assert!(err.title.contains("API Key"));
    }

    #[test]
    fn test_translate_permission_denied() {
        let translator = ErrorTranslator::new();
        let err = translator.translate("Permission denied: admin required");
        assert_eq!(err.code, ErrorCode::PermissionDenied);
        assert_eq!(err.level, ErrorLevel::Hint);
    }

    #[test]
    fn test_format_chat_fatal() {
        let translator = ErrorTranslator::new();
        let humanized = HumanizedError {
            code: ErrorCode::ConfigMissing,
            level: ErrorLevel::Fatal,
            title: "缺少配置".to_string(),
            reason: "test".to_string(),
            suggestion: "fix it".to_string(),
        };
        let formatted = translator.format_chat(&humanized);
        assert!(formatted.contains("💥"));
    }

    #[test]
    fn test_format_chat_hint() {
        let translator = ErrorTranslator::new();
        let humanized = HumanizedError {
            code: ErrorCode::PermissionDenied,
            level: ErrorLevel::Hint,
            title: "权限不够".to_string(),
            reason: "test".to_string(),
            suggestion: "填 admin_id".to_string(),
        };
        let formatted = translator.format_chat(&humanized);
        assert!(formatted.contains("💡"));
    }

    #[test]
    fn test_all_codes_have_templates() {
        let all_codes = vec![
            ErrorCode::ProviderRateLimit,
            ErrorCode::ProviderInvalidKey,
            ErrorCode::ProviderTimeout,
            ErrorCode::ProviderModelNotFound,
            ErrorCode::ProviderContextExceeded,
            ErrorCode::ProviderBalanceEmpty,
            ErrorCode::PlatformConnectionFailed,
            ErrorCode::PlatformAuthFailed,
            ErrorCode::PlatformMessageTooLong,
            ErrorCode::PlatformBanned,
            ErrorCode::DbUniqueConstraint,
            ErrorCode::DbNotFound,
            ErrorCode::DbConnectionFailed,
            ErrorCode::ConfigMissing,
            ErrorCode::ConfigInvalid,
            ErrorCode::PluginNotFound,
            ErrorCode::PluginLoadFailed,
            ErrorCode::PermissionDenied,
            ErrorCode::NetworkError,
            ErrorCode::Unknown,
        ];
        
        for code in all_codes {
            assert!(
                ERROR_TEMPLATES.contains_key(&code),
                "ErrorCode {:?} missing template",
                code
            );
        }
    }
}
