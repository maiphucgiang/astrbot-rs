// Phase 2: UX i18n by Soulclawter
// ErrorTranslator 多语言扩展 — 支持中/英/日三种语言的错误描述

use crate::errors::{ErrorCode, ErrorLevel, HumanizedError};

/// 支持的语言
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Lang {
    Chinese,
    English,
    Japanese,
}

impl Lang {
    /// 从字符串解析语言
    pub fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "zh" | "cn" | "chinese" | "中文" | "zh_cn" => Lang::Chinese,
            "en" | "english" | "英文" => Lang::English,
            "jp" | "ja" | "japanese" | "日文" | "日语" => Lang::Japanese,
            _ => Lang::Chinese,
        }
    }

    /// 返回语言自身的显示名
    pub fn display_name(&self) -> &'static str {
        match self {
            Lang::Chinese => "中文",
            Lang::English => "English",
            Lang::Japanese => "日本語",
        }
    }
}

/// 多语言错误模板
/// (title, reason, suggestion) 三元组
pub struct I18nTemplate {
    pub title: &'static str,
    pub reason: &'static str,
    pub suggestion: &'static str,
}

/// 为给定错误码和语言返回翻译模板
pub fn get_template(code: &ErrorCode, lang: Lang) -> I18nTemplate {
    match lang {
        Lang::Chinese => get_zh_template(code),
        Lang::English => get_en_template(code),
        Lang::Japanese => get_jp_template(code),
    }
}

/// 中文模板（已有，直接复制自 ERROR_TEMPLATES）
fn get_zh_template(code: &ErrorCode) -> I18nTemplate {
    use ErrorCode::*;
    match code {
        ProviderRateLimit => I18nTemplate {
            title: "请求太快了",
            reason: "服务商限制了请求频率",
            suggestion: "等一分钟再试，或者换个模型。",
        },
        ProviderInvalidKey => I18nTemplate {
            title: "API Key 有问题",
            reason: "密钥无效或已过期",
            suggestion: "检查 config 里的 api_key 有没有填对，或者去服务商后台重新生成一个。",
        },
        ProviderTimeout => I18nTemplate {
            title: "模型响应超时",
            reason: "服务商响应太慢或者网络不稳定",
            suggestion: "再试一次，如果经常超时可以考虑换个响应更快的模型。",
        },
        ProviderModelNotFound => I18nTemplate {
            title: "找不到这个模型",
            reason: "模型名称可能打错了，或者服务商已下架",
            suggestion: "用 /model 看看有哪些可用的模型。",
        },
        ProviderContextExceeded => I18nTemplate {
            title: "对话太长了",
            reason: "超过了模型的上下文长度限制",
            suggestion: "用 /reset 重置一下会话记忆，或者开启自动总结。",
        },
        ProviderBalanceEmpty => I18nTemplate {
            title: "余额不足",
            reason: "服务商账户里没有余额了",
            suggestion: "去服务商后台充值，或者切换到免费的模型（如 Ollama 本地部署）。",
        },
        PlatformConnectionFailed => I18nTemplate {
            title: "暂时连不上平台",
            reason: "网络问题或平台服务维护",
            suggestion: "正在自动重试...如果持续失败，检查一下网络连接和平台状态。",
        },
        PlatformAuthFailed => I18nTemplate {
            title: "平台认证失败",
            reason: "Token 过期或权限不足",
            suggestion: "检查一下平台配置里的 token 是否有效，可能需要重新获取。",
        },
        PlatformMessageTooLong => I18nTemplate {
            title: "消息太长了",
            reason: "超过了平台单条消息长度限制",
            suggestion: "把消息拆成几段发送，或者开启自动分段。",
        },
        PlatformBanned => I18nTemplate {
            title: "发送被限制了",
            reason: "平台风控或触发敏感词",
            suggestion: "检查一下消息内容，避免敏感词。等几分钟再试。",
        },
        DbUniqueConstraint => I18nTemplate {
            title: "这条记录已经存在了",
            reason: "数据库里已经有相同的数据",
            suggestion: "如果你是想更新它，用修改功能而不是新建。",
        },
        DbNotFound => I18nTemplate {
            title: "找不到这条记录",
            reason: "数据库里没有你要找的数据",
            suggestion: "检查一下 ID 或名称是否输入正确。",
        },
        DbConnectionFailed => I18nTemplate {
            title: "数据库连不上",
            reason: "SQLite 文件可能被占用或损坏",
            suggestion: "检查一下 data/ 目录权限，或者重启一下服务。",
        },
        ConfigMissing => I18nTemplate {
            title: "缺少配置文件",
            reason: "首次启动或配置文件被删除",
            suggestion: "运行 onboarding 流程自动创建，或者手动复制一份 config.example.yaml。",
        },
        ConfigInvalid => I18nTemplate {
            title: "配置文件格式不对",
            reason: "YAML 语法错误或缺少必填项",
            suggestion: "用 YAML 校验工具检查一下，或者对照文档逐项排查。",
        },
        PluginNotFound => I18nTemplate {
            title: "插件没找到",
            reason: "名字可能打错了，或者还没安装",
            suggestion: "用 /plugin list 看看有哪些可用的，或者 /plugin install 安装新插件。",
        },
        PluginLoadFailed => I18nTemplate {
            title: "插件加载失败",
            reason: "插件依赖缺失或版本不兼容",
            suggestion: "检查一下插件文档的依赖要求，或者尝试更新插件版本。",
        },
        PermissionDenied => I18nTemplate {
            title: "权限不够",
            reason: "这个操作需要管理员权限",
            suggestion: "如果你就是管理员，把 QQ 号/用户 ID 填进 config 的 admin_id 里。",
        },
        NetworkError => I18nTemplate {
            title: "网络出问题了",
            reason: "可能是 DNS、防火墙或代理配置",
            suggestion: "检查一下网络连接，如果有代理检查一下代理设置。",
        },
        Unknown => I18nTemplate {
            title: "出了点问题",
            reason: "未知错误",
            suggestion: "把错误日志发给开发者看看，或者重启一下试试。",
        },
    }
}

/// 英文模板
fn get_en_template(code: &ErrorCode) -> I18nTemplate {
    use ErrorCode::*;
    match code {
        ProviderRateLimit => I18nTemplate {
            title: "Rate limited",
            reason: "The provider is throttling requests",
            suggestion: "Wait a minute and try again, or switch to a different model.",
        },
        ProviderInvalidKey => I18nTemplate {
            title: "Invalid API Key",
            reason: "The key is invalid or expired",
            suggestion: "Check your config file for the correct api_key, or regenerate one at the provider dashboard.",
        },
        ProviderTimeout => I18nTemplate {
            title: "Provider timed out",
            reason: "The provider is responding too slowly or the network is unstable",
            suggestion: "Try again. If timeouts persist, consider switching to a faster model.",
        },
        ProviderModelNotFound => I18nTemplate {
            title: "Model not found",
            reason: "The model name may be misspelled or has been removed",
            suggestion: "Use /model to see available models.",
        },
        ProviderContextExceeded => I18nTemplate {
            title: "Context too long",
            reason: "Exceeded the model's context length limit",
            suggestion: "Use /reset to clear session memory, or enable auto-summarization.",
        },
        ProviderBalanceEmpty => I18nTemplate {
            title: "Insufficient balance",
            reason: "No remaining credits in the provider account",
            suggestion: "Recharge at the provider dashboard, or switch to a free model like Ollama.",
        },
        PlatformConnectionFailed => I18nTemplate {
            title: "Cannot connect to platform",
            reason: "Network issue or platform maintenance",
            suggestion: "Retrying automatically... If it keeps failing, check your network and platform status.",
        },
        PlatformAuthFailed => I18nTemplate {
            title: "Platform authentication failed",
            reason: "Token expired or insufficient permissions",
            suggestion: "Check if the platform token in config is still valid. You may need to re-acquire it.",
        },
        PlatformMessageTooLong => I18nTemplate {
            title: "Message too long",
            reason: "Exceeds the platform's single-message length limit",
            suggestion: "Split the message into multiple parts, or enable auto-segmentation.",
        },
        PlatformBanned => I18nTemplate {
            title: "Sending restricted",
            reason: "Platform risk control or sensitive word triggered",
            suggestion: "Check your message content for sensitive terms. Wait a few minutes and retry.",
        },
        DbUniqueConstraint => I18nTemplate {
            title: "Record already exists",
            reason: "Database already has identical data",
            suggestion: "If you meant to update it, use the edit feature instead of creating a new one.",
        },
        DbNotFound => I18nTemplate {
            title: "Record not found",
            reason: "The data you're looking for does not exist in the database",
            suggestion: "Double-check the ID or name you entered.",
        },
        DbConnectionFailed => I18nTemplate {
            title: "Database connection failed",
            reason: "SQLite file may be locked or corrupted",
            suggestion: "Check the data/ directory permissions, or restart the service.",
        },
        ConfigMissing => I18nTemplate {
            title: "Missing config file",
            reason: "First launch or config file deleted",
            suggestion: "Run the onboarding flow to auto-create one, or manually copy config.example.yaml.",
        },
        ConfigInvalid => I18nTemplate {
            title: "Invalid config format",
            reason: "YAML syntax error or missing required fields",
            suggestion: "Validate with a YAML linter, or check against the documentation.",
        },
        PluginNotFound => I18nTemplate {
            title: "Plugin not found",
            reason: "Name may be misspelled or not yet installed",
            suggestion: "Use /plugin list to see available plugins, or /plugin install to add a new one.",
        },
        PluginLoadFailed => I18nTemplate {
            title: "Plugin load failed",
            reason: "Missing dependencies or version incompatibility",
            suggestion: "Check the plugin's dependency requirements, or try updating the plugin.",
        },
        PermissionDenied => I18nTemplate {
            title: "Permission denied",
            reason: "This operation requires admin privileges",
            suggestion: "If you are the admin, add your QQ/user ID to admin_id in the config.",
        },
        NetworkError => I18nTemplate {
            title: "Network issue",
            reason: "Possible DNS, firewall, or proxy configuration problem",
            suggestion: "Check your network connection. If using a proxy, verify its settings.",
        },
        Unknown => I18nTemplate {
            title: "Something went wrong",
            reason: "Unknown error",
            suggestion: "Send the error log to the developer, or try restarting.",
        },
    }
}

/// 日文模板
fn get_jp_template(code: &ErrorCode) -> I18nTemplate {
    use ErrorCode::*;
    match code {
        ProviderRateLimit => I18nTemplate {
            title: "リクエストが多すぎます",
            reason: "プロバイダがリクエスト制限を設けています",
            suggestion: "1分ほど待ってから再試行するか、別のモデルをお使いください。",
        },
        ProviderInvalidKey => I18nTemplate {
            title: "APIキーが無効です",
            reason: "キーが無効または期限切れです",
            suggestion: "設定ファイルの api_key が正しいか確認するか、プロバイダダッシュボードで再生成してください。",
        },
        ProviderTimeout => I18nTemplate {
            title: "プロバイダ応答タイムアウト",
            reason: "プロバイダの応答が遅いか、ネットワークが不安定です",
            suggestion: "もう一度試してください。頻繁にタイムアウトする場合は、応答速度の速いモデルに変更してください。",
        },
        ProviderModelNotFound => I18nTemplate {
            title: "モデルが見つかりません",
            reason: "モデル名が間違っているか、プロバイダが提供を終了しました",
            suggestion: "/model コマンドで利用可能なモデルを確認してください。",
        },
        ProviderContextExceeded => I18nTemplate {
            title: "会話が長すぎます",
            reason: "モデルのコンテキスト長制限を超えました",
            suggestion: "/reset でセッションメモリをリセットするか、自動要約を有効にしてください。",
        },
        ProviderBalanceEmpty => I18nTemplate {
            title: "残高不足",
            reason: "プロバイダアカウントの残高がゼロです",
            suggestion: "プロバイダダッシュボードでチャージするか、Ollama などの無料モデルに切り替えてください。",
        },
        PlatformConnectionFailed => I18nTemplate {
            title: "プラットフォームに接続できません",
            reason: "ネットワーク問題またはプラットフォームのメンテナンス",
            suggestion: "自動的に再試行しています... 継続して失敗する場合は、ネットワークとプラットフォームの状態を確認してください。",
        },
        PlatformAuthFailed => I18nTemplate {
            title: "プラットフォーム認証失敗",
            reason: "トークンが期限切れか権限不足です",
            suggestion: "設定のプラットフォーントークンが有効か確認してください。再取得が必要な場合があります。",
        },
        PlatformMessageTooLong => I18nTemplate {
            title: "メッセージが長すぎます",
            reason: "プラットフォームの1メッセージ長制限を超えました",
            suggestion: "メッセージを複数に分割するか、自動分割を有効にしてください。",
        },
        PlatformBanned => I18nTemplate {
            title: "送信が制限されています",
            reason: "プラットフォームのリスク管理またはセンシティブワードが検出されました",
            suggestion: "メッセージ内容にセンシティブな言葉がないか確認してください。数分待ってから再試行してください。",
        },
        DbUniqueConstraint => I18nTemplate {
            title: "レコードが既に存在します",
            reason: "データベースに同一のデータが既にあります",
            suggestion: "更新する場合は、新規作成ではなく編集機能をお使いください。",
        },
        DbNotFound => I18nTemplate {
            title: "レコードが見つかりません",
            reason: "データベースに該当するデータがありません",
            suggestion: "入力した ID または名称を再度確認してください。",
        },
        DbConnectionFailed => I18nTemplate {
            title: "データベース接続失敗",
            reason: "SQLite ファイルがロックされているか破損している可能性があります",
            suggestion: "data/ ディレクトリの権限を確認するか、サービスを再起動してください。",
        },
        ConfigMissing => I18nTemplate {
            title: "設定ファイルが見つかりません",
            reason: "初回起動または設定ファイルが削除されました",
            suggestion: "オンボーディングフローを実行して自動作成するか、config.example.yaml を手動でコピーしてください。",
        },
        ConfigInvalid => I18nTemplate {
            title: "設定ファイルの形式が無効です",
            reason: "YAML 構文エラーまたは必須項目が不足しています",
            suggestion: "YAML バリデーションツールで確認するか、ドキュメントと照らして項目を確認してください。",
        },
        PluginNotFound => I18nTemplate {
            title: "プラグインが見つかりません",
            reason: "名前が間違っているか、まだインストールされていません",
            suggestion: "/plugin list で利用可能なプラグインを確認するか、/plugin install で新規追加してください。",
        },
        PluginLoadFailed => I18nTemplate {
            title: "プラグイン読み込み失敗",
            reason: "依存関係が不足しているか、バージョンに互換性がありません",
            suggestion: "プラグインの依存関係要件を確認するか、プラグインの更新を試してください。",
        },
        PermissionDenied => I18nTemplate {
            title: "権限が不足しています",
            reason: "この操作には管理者権限が必要です",
            suggestion: "管理者の場合は、QQ番号/ユーザーIDを設定ファイルの admin_id に追加してください。",
        },
        NetworkError => I18nTemplate {
            title: "ネットワークエラー",
            reason: "DNS、ファイアウォール、またはプロキシ設定の問題の可能性があります",
            suggestion: "ネットワーク接続を確認してください。プロキシを使用している場合は、その設定を確認してください。",
        },
        Unknown => I18nTemplate {
            title: "問題が発生しました",
            reason: "不明なエラー",
            suggestion: "エラーログを開発者に送信するか、再起動を試してください。",
        },
    }
}

/// 用指定语言翻译错误码（供外部直接调用）
pub fn translate_by_code(code: &ErrorCode, level: ErrorLevel, lang: Lang) -> HumanizedError {
    let t = get_template(code, lang);
    HumanizedError {
        code: code.clone(),
        level,
        title: t.title.to_string(),
        reason: t.reason.to_string(),
        suggestion: t.suggestion.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::errors::{ErrorCode, ErrorLevel};

    #[test]
    fn test_lang_from_str() {
        assert_eq!(Lang::from_str("zh"), Lang::Chinese);
        assert_eq!(Lang::from_str("en"), Lang::English);
        assert_eq!(Lang::from_str("ja"), Lang::Japanese);
        assert_eq!(Lang::from_str("JP"), Lang::Japanese);
        assert_eq!(Lang::from_str("中文"), Lang::Chinese);
        assert_eq!(Lang::from_str("unknown"), Lang::Chinese); // fallback
    }

    #[test]
    fn test_translate_by_code_en() {
        let h = translate_by_code(
            &ErrorCode::ProviderInvalidKey,
            ErrorLevel::Error,
            Lang::English,
        );
        assert_eq!(h.title, "Invalid API Key");
        assert!(h.suggestion.contains("dashboard"));
    }

    #[test]
    fn test_translate_by_code_jp() {
        let h = translate_by_code(
            &ErrorCode::ProviderBalanceEmpty,
            ErrorLevel::Error,
            Lang::Japanese,
        );
        assert_eq!(h.title, "残高不足");
        assert!(h.suggestion.contains("Ollama"));
    }

    #[test]
    fn test_translate_by_code_zh() {
        let h = translate_by_code(&ErrorCode::ConfigMissing, ErrorLevel::Fatal, Lang::Chinese);
        assert_eq!(h.title, "缺少配置文件");
        assert_eq!(h.level, ErrorLevel::Fatal);
    }

    #[test]
    fn test_all_codes_covered_zh() {
        let all = vec![
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
        for code in all {
            let h = translate_by_code(&code, ErrorLevel::Error, Lang::Chinese);
            assert!(!h.title.is_empty(), "ZH title empty for {:?}", code);
            assert!(!h.reason.is_empty(), "ZH reason empty for {:?}", code);
            assert!(
                !h.suggestion.is_empty(),
                "ZH suggestion empty for {:?}",
                code
            );
        }
    }

    #[test]
    fn test_all_codes_covered_en() {
        let all = vec![
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
        for code in all {
            let h = translate_by_code(&code, ErrorLevel::Error, Lang::English);
            assert!(!h.title.is_empty(), "EN title empty for {:?}", code);
        }
    }

    #[test]
    fn test_all_codes_covered_jp() {
        let all = vec![
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
        for code in all {
            let h = translate_by_code(&code, ErrorLevel::Error, Lang::Japanese);
            assert!(!h.title.is_empty(), "JP title empty for {:?}", code);
        }
    }
}
