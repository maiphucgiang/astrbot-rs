//! Backup system for AstrBot
//!
//! Provides database + configuration backup and restore functionality,
//! plus lightweight config export/import helpers.

use chrono::{DateTime, Utc};
use serde_json::Value;
use std::path::{Path, PathBuf};
use tokio::fs;

use crate::config::AstrBotConfig;
use crate::errors::{AstrBotError, Result};

/// Information about a single backup
#[derive(Debug, Clone, PartialEq)]
pub struct BackupInfo {
    /// Backup folder name (e.g. backup_20250115_143052)
    pub id: String,
    /// When the backup was created
    pub created_at: DateTime<Utc>,
    /// Size of the database file in bytes
    pub db_size_bytes: u64,
    /// Size of the config JSON file in bytes
    pub config_size_bytes: u64,
    /// Total size of the backup in bytes
    pub total_size_bytes: u64,
}

/// Manages creation, listing, restoration, and cleanup of backups
pub struct BackupManager {
    /// Directory where backups are stored
    backup_dir: PathBuf,
    /// Maximum number of backups to keep (oldest are auto-removed)
    max_backups: usize,
}

impl BackupManager {
    /// Create a new BackupManager
    pub fn new(backup_dir: impl Into<PathBuf>, max_backups: usize) -> Self {
        Self {
            backup_dir: backup_dir.into(),
            max_backups,
        }
    }

    /// Create a new backup of the database and config
    pub async fn create_backup(&self, db_path: &Path, config: &Value) -> Result<BackupInfo> {
        fs::create_dir_all(&self.backup_dir).await.map_err(|e| {
            AstrBotError::Internal(format!("Failed to create backup directory: {}", e))
        })?;

        let timestamp = Utc::now();
        let id = timestamp.format("backup_%Y%m%d_%H%M%S").to_string();
        let backup_path = self.backup_dir.join(&id);

        fs::create_dir_all(&backup_path).await.map_err(|e| {
            AstrBotError::Internal(format!("Failed to create backup subdirectory: {}", e))
        })?;

        let db_backup_path = backup_path.join("astrbot.db");
        fs::copy(db_path, &db_backup_path)
            .await
            .map_err(|e| AstrBotError::Internal(format!("Failed to copy database file: {}", e)))?;

        let db_size_bytes = fs::metadata(&db_backup_path)
            .await
            .map_err(|e| AstrBotError::Internal(format!("Failed to get db metadata: {}", e)))?
            .len();

        let config_backup_path = backup_path.join("config.json");
        let config_json = serde_json::to_string_pretty(config).map_err(|e| {
            AstrBotError::Serialization(format!("Failed to serialize config: {}", e))
        })?;
        fs::write(&config_backup_path, config_json)
            .await
            .map_err(|e| AstrBotError::Internal(format!("Failed to write config backup: {}", e)))?;

        let config_size_bytes = fs::metadata(&config_backup_path)
            .await
            .map_err(|e| AstrBotError::Internal(format!("Failed to get config metadata: {}", e)))?
            .len();

        let total_size_bytes = db_size_bytes + config_size_bytes;
        self.cleanup_old_backups().await?;

        Ok(BackupInfo {
            id,
            created_at: timestamp,
            db_size_bytes,
            config_size_bytes,
            total_size_bytes,
        })
    }

    /// List all available backups
    pub async fn list_backups(&self) -> Result<Vec<BackupInfo>> {
        let mut backups = Vec::new();
        if !self.backup_dir.exists() {
            return Ok(backups);
        }

        let mut entries = fs::read_dir(&self.backup_dir).await.map_err(|e| {
            AstrBotError::Internal(format!("Failed to read backup directory: {}", e))
        })?;

        while let Some(entry) = entries
            .next_entry()
            .await
            .map_err(|e| AstrBotError::Internal(format!("Failed to read directory entry: {}", e)))?
        {
            let path = entry.path();
            let id = match entry.file_name().into_string() {
                Ok(name) => name,
                Err(_) => continue,
            };
            if !path.is_dir() || !id.starts_with("backup_") {
                continue;
            }

            let created_at = if id.len() >= 22 {
                let dp = &id[7..15];
                let tp = &id[16..22];
                let y: i32 = dp[0..4].parse().unwrap_or(1970);
                let m: u32 = dp[4..6].parse().unwrap_or(1);
                let d: u32 = dp[6..8].parse().unwrap_or(1);
                let h: u32 = tp[0..2].parse().unwrap_or(0);
                let mn: u32 = tp[2..4].parse().unwrap_or(0);
                let s: u32 = tp[4..6].parse().unwrap_or(0);
                match chrono::NaiveDate::from_ymd_opt(y, m, d).and_then(|d| d.and_hms_opt(h, mn, s))
                {
                    Some(naive) => DateTime::<Utc>::from_naive_utc_and_offset(naive, chrono::Utc),
                    None => Utc::now(),
                }
            } else {
                Utc::now()
            };

            let db_path = path.join("astrbot.db");
            let config_path = path.join("config.json");
            let db_size = if db_path.exists() {
                fs::metadata(&db_path).await.map(|m| m.len()).unwrap_or(0)
            } else {
                0
            };
            let config_size = if config_path.exists() {
                fs::metadata(&config_path)
                    .await
                    .map(|m| m.len())
                    .unwrap_or(0)
            } else {
                0
            };
            backups.push(BackupInfo {
                id,
                created_at,
                db_size_bytes: db_size,
                config_size_bytes: config_size,
                total_size_bytes: db_size + config_size,
            });
        }
        backups.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        Ok(backups)
    }

    /// Restore a backup
    pub async fn restore_backup(&self, id: &str, target_db_path: &Path) -> Result<()> {
        let backup_path = self.backup_dir.join(id).join("astrbot.db");
        if !backup_path.exists() {
            return Err(AstrBotError::NotFound(format!("Backup '{}' not found", id)));
        }
        if let Some(parent) = target_db_path.parent() {
            fs::create_dir_all(parent).await.map_err(|e| {
                AstrBotError::Internal(format!("Failed to create target dir: {}", e))
            })?;
        }
        fs::copy(&backup_path, target_db_path)
            .await
            .map_err(|e| AstrBotError::Internal(format!("Failed to restore: {}", e)))?;
        Ok(())
    }

    /// Delete a backup
    pub async fn delete_backup(&self, id: &str) -> Result<()> {
        let p = self.backup_dir.join(id);
        if !p.exists() {
            return Err(AstrBotError::NotFound(format!("Backup '{}' not found", id)));
        }
        fs::remove_dir_all(&p)
            .await
            .map_err(|e| AstrBotError::Internal(format!("Failed to delete: {}", e)))?;
        Ok(())
    }

    /// Cleanup old backups
    pub async fn cleanup_old_backups(&self) -> Result<()> {
        if self.max_backups == 0 {
            return Ok(());
        }
        let mut backups = self.list_backups().await?;
        if backups.len() > self.max_backups {
            backups.sort_by(|a, b| a.created_at.cmp(&b.created_at));
            for b in &backups[..backups.len() - self.max_backups] {
                self.delete_backup(&b.id).await?;
            }
        }
        Ok(())
    }
}

// ── Config export/import helpers ──

/// Export `AstrBotConfig` to JSON
pub async fn export_config(config: &AstrBotConfig, path: impl AsRef<Path>) -> Result<PathBuf> {
    let path = path.as_ref();
    let json = serde_json::to_string_pretty(config)
        .map_err(|e| AstrBotError::Serialization(format!("Failed to serialize: {}", e)))?;
    if let Some(p) = path.parent() {
        fs::create_dir_all(p)
            .await
            .map_err(|e| AstrBotError::Internal(format!("Failed to create dir: {}", e)))?;
    }
    fs::write(path, json)
        .await
        .map_err(|e| AstrBotError::Internal(format!("Failed to write: {}", e)))?;
    Ok(path.to_path_buf())
}

/// Import `AstrBotConfig` from JSON with validation
pub async fn import_config(path: impl AsRef<Path>) -> Result<AstrBotConfig> {
    let path = path.as_ref();
    let content = fs::read_to_string(path)
        .await
        .map_err(|e| AstrBotError::Config(format!("Failed to read '{}': {}", path.display(), e)))?;
    let cfg: AstrBotConfig = serde_json::from_str(&content)
        .map_err(|e| AstrBotError::Config(format!("Invalid JSON '{}': {}", path.display(), e)))?;
    if cfg.nickname.trim().is_empty() {
        return Err(AstrBotError::Validation("nickname cannot be empty".into()));
    }
    if cfg.prefixes.is_empty() {
        return Err(AstrBotError::Validation(
            "at least one prefix required".into(),
        ));
    }
    for p in &cfg.prefixes {
        if p.trim().is_empty() {
            return Err(AstrBotError::Validation("prefix cannot be empty".into()));
        }
    }
    Ok(cfg)
}

/// List JSON config files in directory
pub async fn list_backup_configs(dir: impl AsRef<Path>) -> Result<Vec<PathBuf>> {
    let dir = dir.as_ref();
    let mut files = Vec::new();
    if !dir.exists() {
        return Ok(files);
    }
    let mut entries = fs::read_dir(dir)
        .await
        .map_err(|e| AstrBotError::Internal(format!("Failed to read dir: {}", e)))?;
    while let Some(e) = entries
        .next_entry()
        .await
        .map_err(|e| AstrBotError::Internal(format!("Failed to read entry: {}", e)))?
    {
        let p = e.path();
        if p.is_file() && p.extension().and_then(|s| s.to_str()) == Some("json") {
            files.push(p);
        }
    }
    files.sort();
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn test_create_backup() {
        let td = std::env::temp_dir().join(format!("astrbot_bk_{}", std::process::id()));
        let bd = td.join("backups");
        let db = td.join("astrbot.db");
        fs::create_dir_all(&td).await.unwrap();
        fs::write(&db, b"fake sqlite data").await.unwrap();
        let m = BackupManager::new(&bd, 5);
        let info = m.create_backup(&db, &json!({"k": "v"})).await.unwrap();
        assert!(info.id.starts_with("backup_"));
        assert_eq!(info.db_size_bytes, 16);
        assert!(bd.join(&info.id).join("astrbot.db").exists());
        assert!(bd.join(&info.id).join("config.json").exists());
        let _ = fs::remove_dir_all(&td).await;
    }

    #[tokio::test]
    async fn test_list_backups() {
        let td = std::env::temp_dir().join(format!("astrbot_lst_{}", std::process::id()));
        let bd = td.join("backups");
        let db = td.join("astrbot.db");
        fs::create_dir_all(&td).await.unwrap();
        fs::write(&db, b"x").await.unwrap();
        let m = BackupManager::new(&bd, 10);
        let i1 = m.create_backup(&db, &json!({})).await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
        let i2 = m.create_backup(&db, &json!({})).await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
        let i3 = m.create_backup(&db, &json!({})).await.unwrap();
        let b = m.list_backups().await.unwrap();
        assert_eq!(b.len(), 3);
        assert_eq!(b[0].id, i3.id);
        assert_eq!(b[1].id, i2.id);
        assert_eq!(b[2].id, i1.id);
        let _ = fs::remove_dir_all(&td).await;
    }

    #[tokio::test]
    async fn test_restore_backup() {
        let td = std::env::temp_dir().join(format!("astrbot_res_{}", std::process::id()));
        let bd = td.join("backups");
        let orig = td.join("orig.db");
        let rst = td.join("rst.db");
        fs::create_dir_all(&td).await.unwrap();
        fs::write(&orig, b"v1").await.unwrap();
        let m = BackupManager::new(&bd, 5);
        let info = m.create_backup(&orig, &json!({"v": 1})).await.unwrap();
        fs::write(&orig, b"modified").await.unwrap();
        m.restore_backup(&info.id, &rst).await.unwrap();
        assert_eq!(fs::read(&rst).await.unwrap(), b"v1");
        let _ = fs::remove_dir_all(&td).await;
    }

    #[tokio::test]
    async fn test_cleanup_and_delete() {
        let td = std::env::temp_dir().join(format!("astrbot_cln_{}", std::process::id()));
        let bd = td.join("backups");
        let db = td.join("astrbot.db");
        fs::create_dir_all(&td).await.unwrap();
        fs::write(&db, b"x").await.unwrap();
        let m = BackupManager::new(&bd, 3);
        let mut ids = Vec::new();
        for _ in 0..5 {
            ids.push(m.create_backup(&db, &json!({})).await.unwrap().id);
            tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
        }
        let b = m.list_backups().await.unwrap();
        assert_eq!(b.len(), 3);
        assert!(!b.iter().any(|bk| bk.id == ids[0]));
        assert!(!b.iter().any(|bk| bk.id == ids[1]));
        m.delete_backup(&ids[2]).await.unwrap();
        assert!(!bd.join(&ids[2]).exists());
        let _ = fs::remove_dir_all(&td).await;
    }

    #[tokio::test]
    async fn test_export_config() {
        let td = std::env::temp_dir().join(format!("astrbot_exp_{}", std::process::id()));
        let ep = td.join("exports").join("cfg.json");
        fs::create_dir_all(&td).await.unwrap();
        let cfg = AstrBotConfig {
            nickname: "TBot".into(),
            prefixes: vec!["/".into(), "!".into()],
            ..Default::default()
        };
        assert!(export_config(&cfg, &ep).await.is_ok());
        assert!(ep.exists());
        let c = fs::read_to_string(&ep).await.unwrap();
        let v: serde_json::Value = serde_json::from_str(&c).unwrap();
        assert_eq!(v["nickname"], "TBot");
        assert_eq!(v["prefixes"], json!(["/", "!"]));
        let _ = fs::remove_dir_all(&td).await;
    }

    #[tokio::test]
    async fn test_import_config() {
        let td = std::env::temp_dir().join(format!("astrbot_imp_{}", std::process::id()));
        let ip = td.join("cfg.json");
        fs::create_dir_all(&td).await.unwrap();
        let orig = AstrBotConfig {
            nickname: "IBot".into(),
            prefixes: vec![">".into()],
            log_level: "debug".into(),
            ..Default::default()
        };
        fs::write(&ip, serde_json::to_string_pretty(&orig).unwrap())
            .await
            .unwrap();
        let loaded = import_config(&ip).await.unwrap();
        assert_eq!(loaded.nickname, "IBot");
        assert_eq!(loaded.prefixes, vec![">".to_string()]);
        assert_eq!(loaded.log_level, "debug");
        let _ = fs::remove_dir_all(&td).await;
    }

    #[tokio::test]
    async fn test_import_validation() {
        let td = std::env::temp_dir().join(format!("astrbot_iv_{}", std::process::id()));
        let bp = td.join("bad.json");
        fs::create_dir_all(&td).await.unwrap();
        // empty nickname
        fs::write(
            &bp,
            json!({"nickname":" ","prefixes":["/"],"log_level":"info"}).to_string(),
        )
        .await
        .unwrap();
        let err = import_config(&bp).await.unwrap_err().to_string();
        println!("DEBUG empty nickname err: {}", err);
        assert!(err.contains("nickname"), "expected 'nickname' in: {}", err);
        // empty prefixes
        fs::write(
            &bp,
            json!({"nickname":"B","prefixes":[],"log_level":"info"}).to_string(),
        )
        .await
        .unwrap();
        assert!(import_config(&bp)
            .await
            .unwrap_err()
            .to_string()
            .contains("prefix"));
        // empty prefix string
        fs::write(
            &bp,
            json!({"nickname":"B","prefixes":[""],"log_level":"info"}).to_string(),
        )
        .await
        .unwrap();
        assert!(import_config(&bp)
            .await
            .unwrap_err()
            .to_string()
            .contains("prefix"));
        // bad json
        fs::write(&bp, b"not json").await.unwrap();
        assert!(import_config(&bp)
            .await
            .unwrap_err()
            .to_string()
            .contains("Invalid"));
        let _ = fs::remove_dir_all(&td).await;
    }

    #[tokio::test]
    async fn test_list_backup_configs() {
        let td = std::env::temp_dir().join(format!("astrbot_lbc_{}", std::process::id()));
        fs::create_dir_all(&td).await.unwrap();
        fs::write(td.join("a.json"), b"{}").await.unwrap();
        fs::write(td.join("b.json"), b"[]").await.unwrap();
        fs::write(td.join("n.txt"), b"x").await.unwrap();
        fs::create_dir(td.join("d")).await.unwrap();
        let f = list_backup_configs(&td).await.unwrap();
        assert_eq!(f.len(), 2);
        assert!(f.iter().any(|p| p.file_name().unwrap() == "a.json"));
        assert!(f.iter().any(|p| p.file_name().unwrap() == "b.json"));
        assert!(list_backup_configs(td.join("none"))
            .await
            .unwrap()
            .is_empty());
        let _ = fs::remove_dir_all(&td).await;
    }
}
