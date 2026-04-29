use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    ReadMessages,
    SendMessages,
    ReadOwnConfig,
    Network {
        allow_hosts: Vec<String>,
        allow_methods: Vec<String>,
    },
    FileSystem {
        read_paths: Vec<String>,
        write_paths: Vec<String>,
    },
    ExecuteCode,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum RiskLevel {
    Low,
    Medium,
    High,
    Critical,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginManifest {
    pub id: String,
    pub name: String,
    pub version: String,
    pub author: String,
    pub checksum: String,
    pub capabilities: Vec<Capability>,
    pub description: String,
}

impl PluginManifest {
    pub fn risk_level(&self) -> RiskLevel {
        let has_network = self
            .capabilities
            .iter()
            .any(|c| matches!(c, Capability::Network { .. }));
        let has_fs = self
            .capabilities
            .iter()
            .any(|c| matches!(c, Capability::FileSystem { .. }));
        let has_exec = self
            .capabilities
            .iter()
            .any(|c| matches!(c, Capability::ExecuteCode));

        if has_exec {
            RiskLevel::Critical
        } else if has_network && has_fs {
            RiskLevel::High
        } else if has_network || has_fs {
            RiskLevel::Medium
        } else {
            RiskLevel::Low
        }
    }

    pub fn is_developer_mode_required(&self) -> bool {
        self.risk_level() == RiskLevel::Critical
    }
}

/// 安装权限检查
pub fn check_install_permission(
    manifest: &PluginManifest,
    developer_mode: bool,
) -> Result<(), String> {
    let risk = manifest.risk_level();
    if risk == RiskLevel::Critical && !developer_mode {
        return Err(format!(
            "Plugin '{}' requires ExecuteCode capability. Enable developer mode to install.",
            manifest.name
        ));
    }
    Ok(())
}

/// 校验插件能力是否允许某操作
pub fn check_capability(manifest: &PluginManifest, required: &Capability) -> Result<(), String> {
    let has = manifest.capabilities.iter().any(|c| match (c, required) {
        (Capability::ReadMessages, Capability::ReadMessages) => true,
        (Capability::SendMessages, Capability::SendMessages) => true,
        (Capability::ReadOwnConfig, Capability::ReadOwnConfig) => true,
        (Capability::ExecuteCode, Capability::ExecuteCode) => true,
        (Capability::Network { .. }, Capability::Network { .. }) => true,
        (Capability::FileSystem { .. }, Capability::FileSystem { .. }) => true,
        _ => false,
    });

    if !has {
        return Err(format!(
            "Plugin '{}' lacks required capability: {:?}",
            manifest.id, required
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_manifest(caps: Vec<Capability>) -> PluginManifest {
        PluginManifest {
            id: "test.plugin".to_string(),
            name: "Test Plugin".to_string(),
            version: "1.0.0".to_string(),
            author: "test".to_string(),
            checksum: "abc123".to_string(),
            capabilities: caps,
            description: "Test".to_string(),
        }
    }

    #[test]
    fn test_risk_levels() {
        let low = make_manifest(vec![Capability::ReadMessages]);
        assert_eq!(low.risk_level(), RiskLevel::Low);

        let high = make_manifest(vec![
            Capability::Network {
                allow_hosts: vec![],
                allow_methods: vec![],
            },
            Capability::FileSystem {
                read_paths: vec![],
                write_paths: vec![],
            },
        ]);
        assert_eq!(high.risk_level(), RiskLevel::High);

        let critical = make_manifest(vec![Capability::ExecuteCode]);
        assert_eq!(critical.risk_level(), RiskLevel::Critical);
    }

    #[test]
    fn test_developer_mode_required() {
        let critical = make_manifest(vec![Capability::ExecuteCode]);
        assert!(critical.is_developer_mode_required());
        assert!(check_install_permission(&critical, false).is_err());
        assert!(check_install_permission(&critical, true).is_ok());
    }
}
