//! Internationalization (i18n) support for AstrBot
//!
//! Provides a simple string-dictionary i18n system with fallback support.

use serde_json::Value;
use std::collections::HashMap;

/// A translation pack: `key -> translated string`.
pub type TranslationPack = HashMap<String, String>;

/// Simple in-memory i18n manager.
#[derive(Debug, Default, Clone)]
pub struct I18nManager {
    packs: HashMap<String, TranslationPack>,
}

impl I18nManager {
    /// Create an empty manager.
    pub fn new() -> Self {
        Self {
            packs: HashMap::new(),
        }
    }

    /// Create a manager pre-loaded with default `zh` and `en` packs.
    pub fn with_defaults() -> Self {
        let mut manager = Self::new();
        manager.load_pack("zh", default_zh_pack());
        manager.load_pack("en", default_en_pack());
        manager
    }

    /// Load a translation pack for a language.
    pub fn load_pack(&mut self, lang: &str, pack: TranslationPack) {
        self.packs.insert(lang.to_string(), pack);
    }

    /// Load a translation pack from a JSON string.
    ///
    /// The JSON should be an object where keys are translation keys and values are strings.
    pub fn load_json(&mut self, lang: &str, json: &str) -> crate::Result<()> {
        let value: Value = serde_json::from_str(json)
            .map_err(|e| crate::AstrBotError::Serialization(e.to_string()))?;
        let mut pack = TranslationPack::new();
        if let Value::Object(map) = value {
            for (k, v) in map {
                if let Value::String(s) = v {
                    pack.insert(k, s);
                }
            }
        }
        self.load_pack(lang, pack);
        Ok(())
    }

    /// Get a translation by language and key.
    ///
    /// Returns `None` if the language or key is missing.
    pub fn get(&self, lang: &str, key: &str) -> Option<&str> {
        self.packs
            .get(lang)
            .and_then(|pack| pack.get(key))
            .map(|s| s.as_str())
    }

    /// Get a translation with fallback to a default language.
    ///
    /// Falls back to `default_lang` if `lang` is missing the key.
    /// If the default language also lacks the key, returns the key itself as a last resort.
    pub fn get_or_default<'a>(&'a self, lang: &str, key: &'a str, default_lang: &str) -> &'a str {
        self.get(lang, key)
            .or_else(|| self.get(default_lang, key))
            .unwrap_or(key)
    }
}

/// Default Chinese translation pack.
fn default_zh_pack() -> TranslationPack {
    let mut pack = TranslationPack::new();
    pack.insert("welcome".to_string(), "欢迎使用 AstrBot！".to_string());
    pack.insert("error".to_string(), "出错了，请稍后再试。".to_string());
    pack.insert("help".to_string(), "发送 /help 查看帮助信息。".to_string());
    pack.insert("unknown_command".to_string(), "未知命令，请检查输入。".to_string());
    pack.insert("rate_limited".to_string(), "请求过于频繁，请稍后再试。".to_string());
    pack.insert("permission_denied".to_string(), "权限不足，无法执行该操作。".to_string());
    pack.insert("not_found".to_string(), "未找到相关内容。".to_string());
    pack.insert("success".to_string(), "操作成功！".to_string());
    pack.insert("loading".to_string(), "正在处理中，请稍候……".to_string());
    pack.insert("cancelled".to_string(), "操作已取消。".to_string());
    pack.insert("timeout".to_string(), "请求超时，请重试。".to_string());
    pack.insert("invalid_input".to_string(), "输入格式不正确。".to_string());
    pack
}

/// Default English translation pack.
fn default_en_pack() -> TranslationPack {
    let mut pack = TranslationPack::new();
    pack.insert("welcome".to_string(), "Welcome to AstrBot!".to_string());
    pack.insert("error".to_string(), "Something went wrong, please try again later.".to_string());
    pack.insert("help".to_string(), "Send /help for help information.".to_string());
    pack.insert("unknown_command".to_string(), "Unknown command, please check your input.".to_string());
    pack.insert("rate_limited".to_string(), "Too many requests, please try again later.".to_string());
    pack.insert("permission_denied".to_string(), "Permission denied.".to_string());
    pack.insert("not_found".to_string(), "No related content found.".to_string());
    pack.insert("success".to_string(), "Operation successful!".to_string());
    pack.insert("loading".to_string(), "Processing, please wait...".to_string());
    pack.insert("cancelled".to_string(), "Operation cancelled.".to_string());
    pack.insert("timeout".to_string(), "Request timed out, please retry.".to_string());
    pack.insert("invalid_input".to_string(), "Invalid input format.".to_string());
    pack
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_i18n_get_and_fallback() {
        let i18n = I18nManager::with_defaults();

        // Direct hit
        assert_eq!(i18n.get("en", "welcome"), Some("Welcome to AstrBot!"));
        assert_eq!(i18n.get("zh", "welcome"), Some("欢迎使用 AstrBot！"));

        // Missing key falls back to default_lang
        assert_eq!(
            i18n.get_or_default("zh", "welcome", "en"),
            "欢迎使用 AstrBot！"
        );

        // Missing language falls back to default
        assert_eq!(
            i18n.get_or_default("fr", "welcome", "en"),
            "Welcome to AstrBot!"
        );

        // Missing key in both -> returns key itself
        assert_eq!(i18n.get_or_default("en", "missing_key", "zh"), "missing_key");
    }

    #[test]
    fn test_i18n_load_json() {
        let mut i18n = I18nManager::new();
        let json = r#"{"greeting": "Hello", "farewell": "Goodbye"}"#;
        i18n.load_json("en", json).unwrap();

        assert_eq!(i18n.get("en", "greeting"), Some("Hello"));
        assert_eq!(i18n.get("en", "farewell"), Some("Goodbye"));
        assert_eq!(i18n.get("en", "missing"), None);
    }

    #[test]
    fn test_i18n_all_default_keys_present() {
        let i18n = I18nManager::with_defaults();
        let keys = vec![
            "welcome",
            "error",
            "help",
            "unknown_command",
            "rate_limited",
            "permission_denied",
            "not_found",
            "success",
            "loading",
            "cancelled",
            "timeout",
            "invalid_input",
        ];

        for key in &keys {
            assert!(
                i18n.get("zh", key).is_some(),
                "zh pack missing key: {}",
                key
            );
            assert!(
                i18n.get("en", key).is_some(),
                "en pack missing key: {}",
                key
            );
        }
    }
}
