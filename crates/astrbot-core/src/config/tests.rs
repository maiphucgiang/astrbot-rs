//! Tests for astrbot-core configuration

#[cfg(test)]
mod tests {
    use astrbot_core::config::*;
    use std::collections::HashMap;

    #[test]
    fn test_default_config() {
        let config = AstrBotConfig::default();
        assert_eq!(config.nickname, "AstrBot");
        assert_eq!(config.prefixes, vec!["/"]);
        assert!(config.admins.is_empty());
        assert_eq!(config.log_level, "info");
    }

    #[test]
    fn test_is_admin() {
        let mut config = AstrBotConfig::default();
        config.admins.push("user123".to_string());

        assert!(config.is_admin("user123"));
        assert!(!config.is_admin("user456"));
    }

    #[test]
    fn test_plugin_config() {
        let mut config = AstrBotConfig::default();
        let plugin_cfg = serde_json::json!({"enabled": true, "key": "value" });
        config.plugins.insert("test_plugin".to_string(), plugin_cfg.clone());

        assert_eq!(config.plugin_config("test_plugin"), Some(&plugin_cfg));
        assert_eq!(config.plugin_config("missing"), None);
    }

    #[test]
    fn test_webui_config_default() {
        let config = WebUiConfig::default();
        assert!(config.enabled);
        assert_eq!(config.host, "0.0.0.0");
        assert_eq!(config.port, 6185);
    }

    #[tokio::test]
    async fn test_config_json_roundtrip() {
        let config = AstrBotConfig::default();
        let temp_file = "/tmp/test_astrbot_config.json";
        
        config.to_file(temp_file).await.unwrap();
        let loaded = AstrBotConfig::from_file(temp_file).await.unwrap();
        
        assert_eq!(loaded.nickname, config.nickname);
        assert_eq!(loaded.prefixes, config.prefixes);
        assert_eq!(loaded.log_level, config.log_level);
        
        tokio::fs::remove_file(temp_file).await.unwrap();
    }

    #[tokio::test]
    async fn test_config_yaml_roundtrip() {
        let config = AstrBotConfig::default();
        let temp_file = "/tmp/test_astrbot_config.yaml";
        
        config.to_file(temp_file).await.unwrap();
        let loaded = AstrBotConfig::from_file(temp_file).await.unwrap();
        
        assert_eq!(loaded.nickname, config.nickname);
        
        tokio::fs::remove_file(temp_file).await.unwrap();
    }
}
