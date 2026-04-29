use crate::config::AstrBotConfig;
use crate::errors::Result;
use serde_json::Value;
use std::collections::HashMap;
use tracing::{info, warn};

/// Apply environment variable overrides to config
///
/// Supports `ASTRBOT_<SECTION>_<KEY>` format:
/// - `ASTRBOT_NICKNAME` → `nickname`
/// - `ASTRBOT_LOG_LEVEL` → `log_level`
/// - `ASTRBOT_DATABASE_URL` → `database_url`
/// - `ASTRBOT_WEBUI_PORT` → `webui.port`
/// - `ASTRBOT_PROVIDERS_0_API_KEY` → `providers[0].api_key`
/// - `ASTRBOT_ADMINS` → `admins` (comma-separated list)
/// - `ASTRBOT_PREFIXES` → `prefixes` (comma-separated list)
pub fn apply_env_overrides(config: &mut AstrBotConfig) -> Result<()> {
    let env_vars: HashMap<String, String> = std::env::vars()
        .filter(|(k, _)| k.starts_with("ASTRBOT_"))
        .collect();

    if env_vars.is_empty() {
        return Ok(());
    }

    info!("Applying {} environment variable overrides", env_vars.len());

    for (key, value) in env_vars {
        let path = key.strip_prefix("ASTRBOT_").unwrap_or(&key);
        let path_lower = path.to_lowercase();

        match path_lower.as_str() {
            "nickname" => {
                config.nickname = value;
                info!("Overridden nickname via env");
            }
            "log_level" => {
                config.log_level = value;
                info!("Overridden log_level via env");
            }
            "database_url" => {
                config.database_url = value;
                info!("Overridden database_url via env");
            }
            "admins" => {
                config.admins = value.split(',').map(|s| s.trim().to_string()).collect();
                info!("Overridden admins via env");
            }
            "prefixes" => {
                config.prefixes = value.split(',').map(|s| s.trim().to_string()).collect();
                info!("Overridden prefixes via env");
            }
            _ => {
                // Try nested paths like WEBUI_PORT, PROVIDERS_0_API_KEY
                if let Some(rest) = path_lower.strip_prefix("webui_") {
                    apply_webui_override(config, rest, &value);
                } else if path_lower.starts_with("providers_") {
                    apply_provider_override(config, path, &value);
                } else if path_lower.starts_with("platforms_") {
                    apply_platform_override(config, path, &value);
                } else {
                    // Store in extra
                    config.extra.insert(path_lower, Value::String(value));
                    info!("Stored {} in extra config", key);
                }
            }
        }
    }

    Ok(())
}

fn apply_webui_override(config: &mut AstrBotConfig, key: &str, value: &str) {
    match key {
        "port" => {
            if let Ok(port) = value.parse::<u16>() {
                config.webui.port = port;
                info!("Overridden webui.port via env");
            } else {
                warn!("Invalid WEBUI_PORT value: {}", value);
            }
        }
        "host" => {
            config.webui.host = value.to_string();
            info!("Overridden webui.host via env");
        }
        "enabled" => {
            config.webui.enabled = value.parse().unwrap_or(true);
            info!("Overridden webui.enabled via env");
        }
        "jwt_secret" => {
            config.webui.jwt_secret = value.to_string();
            info!("Overridden webui.jwt_secret via env");
        }
        _ => {
            warn!("Unknown webui override: {}", key);
        }
    }
}

fn apply_provider_override(config: &mut AstrBotConfig, path: &str, value: &str) {
    // path format: PROVIDERS_0_API_KEY or PROVIDERS_0_BASE_URL
    let parts: Vec<&str> = path.split('_').collect();
    if parts.len() < 3 {
        warn!("Invalid provider override path: {}", path);
        return;
    }

    // Try to parse index
    if let Ok(index) = parts[1].parse::<usize>() {
        if index < config.providers.len() {
            let field = parts[2..].join("_").to_lowercase();
            match field.as_str() {
                "api_key" => {
                    config.providers[index].api_key = Some(value.to_string());
                    info!("Overridden providers[{}].api_key via env", index);
                }
                "base_url" => {
                    config.providers[index].base_url = Some(value.to_string());
                    info!("Overridden providers[{}].base_url via env", index);
                }
                "model" => {
                    config.providers[index].model = value.to_string();
                    info!("Overridden providers[{}].model via env", index);
                }
                "enabled" => {
                    config.providers[index].enabled = value.parse().unwrap_or(true);
                    info!("Overridden providers[{}].enabled via env", index);
                }
                _ => {
                    config.providers[index]
                        .extra
                        .insert(field.clone(), Value::String(value.to_string()));
                    info!("Stored providers[{}].{} in extra via env", index, field);
                }
            }
        } else {
            warn!(
                "Provider index {} out of range (max: {})",
                index,
                config.providers.len()
            );
        }
    } else {
        warn!("Invalid provider index in path: {}", path);
    }
}

fn apply_platform_override(config: &mut AstrBotConfig, path: &str, value: &str) {
    let parts: Vec<&str> = path.split('_').collect();
    if parts.len() < 3 {
        warn!("Invalid platform override path: {}", path);
        return;
    }

    if let Ok(index) = parts[1].parse::<usize>() {
        if index < config.platforms.len() {
            let field = parts[2..].join("_").to_lowercase();
            config.platforms[index]
                .config
                .insert(field, Value::String(value.to_string()));
            info!("Overridden platforms[{}].config via env", index);
        } else {
            warn!(
                "Platform index {} out of range (max: {})",
                index,
                config.platforms.len()
            );
        }
    }
}

/// Build config from file + env overrides
pub async fn load_config_with_env<P: AsRef<std::path::Path>>(path: P) -> Result<AstrBotConfig> {
    let mut config = AstrBotConfig::from_file(path).await?;
    apply_env_overrides(&mut config)?;
    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_env_override_nickname() {
        let mut config = AstrBotConfig::default();
        unsafe {
            std::env::set_var("ASTRBOT_NICKNAME", "TestBot");
        }
        apply_env_overrides(&mut config).unwrap();
        assert_eq!(config.nickname, "TestBot");
    }

    #[test]
    fn test_env_override_admins() {
        let mut config = AstrBotConfig::default();
        unsafe {
            std::env::set_var("ASTRBOT_ADMINS", "user1,user2,user3");
        }
        apply_env_overrides(&mut config).unwrap();
        assert_eq!(config.admins, vec!["user1", "user2", "user3"]);
    }

    #[test]
    fn test_env_override_webui_port() {
        let mut config = AstrBotConfig::default();
        unsafe {
            std::env::set_var("ASTRBOT_WEBUI_PORT", "8080");
        }
        apply_env_overrides(&mut config).unwrap();
        assert_eq!(config.webui.port, 8080);
    }
}
