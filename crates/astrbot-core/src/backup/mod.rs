//! Backup system for AstrBot
//!
//! Provides database + configuration backup and restore functionality.

use std::path::{Path, PathBuf};
use chrono::{DateTime, Utc};
use serde_json::Value;
use tokio::fs;

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
    ///
    /// The backup is stored in a timestamped subdirectory under `backup_dir`.
    pub async fn create_backup(&self, db_path: &Path, config: &Value) -> Result<BackupInfo> {
        // Ensure backup directory exists
        fs::create_dir_all(&self.backup_dir)
            .await
            .map_err(|e| AstrBotError::Internal(format!("Failed to create backup directory: {}", e)))?;

        let timestamp = Utc::now();
        let id = timestamp.format("backup_%Y%m%d_%H%M%S").to_string();
        let backup_path = self.backup_dir.join(&id);

        // Create backup subdirectory
        fs::create_dir_all(&backup_path)
            .await
            .map_err(|e| AstrBotError::Internal(format!("Failed to create backup subdirectory: {}", e)))?;

        // Copy database file
        let db_backup_path = backup_path.join("astrbot.db");
        fs::copy(db_path, &db_backup_path)
            .await
            .map_err(|e| AstrBotError::Internal(format!("Failed to copy database file: {}", e)))?;

        let db_size_bytes = fs::metadata(&db_backup_path)
            .await
            .map_err(|e| AstrBotError::Internal(format!("Failed to get db metadata: {}", e)))?
            .len();

        // Write config JSON
        let config_backup_path = backup_path.join("config.json");
        let config_json = serde_json::to_string_pretty(config)
            .map_err(|e| AstrBotError::Serialization(format!("Failed to serialize config: {}", e)))?;
        fs::write(&config_backup_path, config_json)
            .await
            .map_err(|e| AstrBotError::Internal(format!("Failed to write config backup: {}", e)))?;

        let config_size_bytes = fs::metadata(&config_backup_path)
            .await
            .map_err(|e| AstrBotError::Internal(format!("Failed to get config metadata: {}", e)))?
            .len();

        let total_size_bytes = db_size_bytes + config_size_bytes;

        // Auto cleanup if we exceed max_backups
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

        // Ensure directory exists (empty list if not)
        if !self.backup_dir.exists() {
            return Ok(backups);
        }

        let mut entries = fs::read_dir(&self.backup_dir)
            .await
            .map_err(|e| AstrBotError::Internal(format!("Failed to read backup directory: {}", e)))?;

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

            // Only process directories that look like our backup folders
            if !path.is_dir() || !id.starts_with("backup_") {
                continue;
            }

            // Parse timestamp from folder name: backup_YYYYMMDD_HHMMSS
            let created_at = if id.len() >= 22 {
                let date_part = &id[7..15]; // YYYYMMDD
                let time_part = &id[16..22]; // HHMMSS
                let year: i32 = date_part[0..4].parse().unwrap_or(1970);
                let month: u32 = date_part[4..6].parse().unwrap_or(1);
                let day: u32 = date_part[6..8].parse().unwrap_or(1);
                let hour: u32 = time_part[0..2].parse().unwrap_or(0);
                let minute: u32 = time_part[2..4].parse().unwrap_or(0);
                let second: u32 = time_part[4..6].parse().unwrap_or(0);
                match chrono::NaiveDate::from_ymd_opt(year, month, day)
                    .and_then(|d| d.and_hms_opt(hour, minute, second))
                {
                    Some(naive) => DateTime::<Utc>::from_naive_utc_and_offset(naive, chrono::Utc),
                    None => Utc::now(),
                }
            } else {
                Utc::now()
            };

            let db_path = path.join("astrbot.db");
            let config_path = path.join("config.json");

            let db_size_bytes = if db_path.exists() {
                fs::metadata(&db_path)
                    .await
                    .map(|m| m.len())
                    .unwrap_or(0)
            } else {
                0
            };

            let config_size_bytes = if config_path.exists() {
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
                db_size_bytes,
                config_size_bytes,
                total_size_bytes: db_size_bytes + config_size_bytes,
            });
        }

        // Sort by created_at descending (newest first)
        backups.sort_by(|a, b| b.created_at.cmp(&a.created_at));

        Ok(backups)
    }

    /// Restore a backup by copying the database file back to the target path
    pub async fn restore_backup(&self, id: &str, target_db_path: &Path) -> Result<()> {
        let backup_path = self.backup_dir.join(id).join("astrbot.db");

        if !backup_path.exists() {
            return Err(AstrBotError::NotFound(format!("Backup '{}' not found", id)));
        }

        // Ensure target directory exists
        if let Some(parent) = target_db_path.parent() {
            fs::create_dir_all(parent)
                .await
                .map_err(|e| AstrBotError::Internal(format!("Failed to create target directory: {}", e)))?;
        }

        fs::copy(&backup_path, target_db_path)
            .await
            .map_err(|e| AstrBotError::Internal(format!("Failed to restore database: {}", e)))?;

        Ok(())
    }

    /// Delete a backup by removing its directory
    pub async fn delete_backup(&self, id: &str) -> Result<()> {
        let backup_path = self.backup_dir.join(id);

        if !backup_path.exists() {
            return Err(AstrBotError::NotFound(format!("Backup '{}' not found", id)));
        }

        fs::remove_dir_all(&backup_path)
            .await
            .map_err(|e| AstrBotError::Internal(format!("Failed to delete backup: {}", e)))?;

        Ok(())
    }

    /// Remove oldest backups when total exceeds `max_backups`
    pub async fn cleanup_old_backups(&self) -> Result<()> {
        if self.max_backups == 0 {
            return Ok(());
        }

        let mut backups = self.list_backups().await?;

        if backups.len() > self.max_backups {
            // Sort oldest first for cleanup
            backups.sort_by(|a, b| a.created_at.cmp(&b.created_at));
            let to_remove = backups.len() - self.max_backups;

            for backup in backups.iter().take(to_remove) {
                self.delete_backup(&backup.id).await?;
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tokio::fs;

    #[tokio::test]
    async fn test_create_backup() {
        let temp_dir = std::env::temp_dir().join(format!("astrbot_backup_test_{}", std::process::id()));
        let backup_dir = temp_dir.join("backups");
        let db_path = temp_dir.join("astrbot.db");

        // Create a fake db file
        fs::create_dir_all(&temp_dir).await.unwrap();
        fs::write(&db_path, b"fake sqlite data").await.unwrap();

        let manager = BackupManager::new(&backup_dir, 5);
        let config = json!({"key": "value", "number": 42});

        let info = manager.create_backup(&db_path, &config).await.unwrap();

        assert!(info.id.starts_with("backup_"));
        assert_eq!(info.db_size_bytes, 16); // "fake sqlite data".len()
        assert!(info.config_size_bytes > 0);
        assert_eq!(info.total_size_bytes, info.db_size_bytes + info.config_size_bytes);

        // Verify files exist
        assert!(backup_dir.join(&info.id).join("astrbot.db").exists());
        assert!(backup_dir.join(&info.id).join("config.json").exists());

        // Cleanup
        let _ = fs::remove_dir_all(&temp_dir).await;
    }

    #[tokio::test]
    async fn test_list_backups() {
        let temp_dir = std::env::temp_dir().join(format!("astrbot_list_test_{}", std::process::id()));
        let backup_dir = temp_dir.join("backups");
        let db_path = temp_dir.join("astrbot.db");

        fs::create_dir_all(&temp_dir).await.unwrap();
        fs::write(&db_path, b"db content").await.unwrap();

        let manager = BackupManager::new(&backup_dir, 10);
        let config = json!({});

        // Create 3 backups with small delays to ensure different timestamps
        let info1 = manager.create_backup(&db_path, &config).await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
        let info2 = manager.create_backup(&db_path, &config).await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
        let info3 = manager.create_backup(&db_path, &config).await.unwrap();

        let backups = manager.list_backups().await.unwrap();
        assert_eq!(backups.len(), 3);

        // Should be sorted newest first
        assert_eq!(backups[0].id, info3.id);
        assert_eq!(backups[1].id, info2.id);
        assert_eq!(backups[2].id, info1.id);

        // Cleanup
        let _ = fs::remove_dir_all(&temp_dir).await;
    }

    #[tokio::test]
    async fn test_restore_backup() {
        let temp_dir = std::env::temp_dir().join(format!("astrbot_restore_test_{}", std::process::id()));
        let backup_dir = temp_dir.join("backups");
        let original_db = temp_dir.join("original.db");
        let restored_db = temp_dir.join("restored.db");

        fs::create_dir_all(&temp_dir).await.unwrap();
        fs::write(&original_db, b"original database content v1").await.unwrap();

        let manager = BackupManager::new(&backup_dir, 5);
        let config = json!({"version": 1});

        let info = manager.create_backup(&original_db, &config).await.unwrap();

        // Modify original
        fs::write(&original_db, b"modified database content").await.unwrap();

        // Restore
        manager.restore_backup(&info.id, &restored_db).await.unwrap();

        // Verify restored matches the backed-up version
        let restored_content = fs::read(&restored_db).await.unwrap();
        assert_eq!(restored_content, b"original database content v1");

        // Cleanup
        let _ = fs::remove_dir_all(&temp_dir).await;
    }

    #[tokio::test]
    async fn test_cleanup_old_backups() {
        let temp_dir = std::env::temp_dir().join(format!("astrbot_cleanup_test_{}", std::process::id()));
        let backup_dir = temp_dir.join("backups");
        let db_path = temp_dir.join("astrbot.db");

        fs::create_dir_all(&temp_dir).await.unwrap();
        fs::write(&db_path, b"x").await.unwrap();

        // max_backups = 3
        let manager = BackupManager::new(&backup_dir, 3);
        let config = json!({});

        // Create 5 backups
        let mut ids = Vec::new();
        for _ in 0..5 {
            let info = manager.create_backup(&db_path, &config).await.unwrap();
            ids.push(info.id);
            tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
        }

        let backups = manager.list_backups().await.unwrap();
        assert_eq!(backups.len(), 3);

        // Oldest 2 should be gone
        let remaining_ids: Vec<String> = backups.iter().map(|b| b.id.clone()).collect();
        assert!(!remaining_ids.contains(&ids[0]));
        assert!(!remaining_ids.contains(&ids[1]));
        assert!(remaining_ids.contains(&ids[2]));
        assert!(remaining_ids.contains(&ids[3]));
        assert!(remaining_ids.contains(&ids[4]));

        // Cleanup
        let _ = fs::remove_dir_all(&temp_dir).await;
    }

    #[tokio::test]
    async fn test_delete_backup() {
        let temp_dir = std::env::temp_dir().join(format!("astrbot_delete_test_{}", std::process::id()));
        let backup_dir = temp_dir.join("backups");
        let db_path = temp_dir.join("astrbot.db");

        fs::create_dir_all(&temp_dir).await.unwrap();
        fs::write(&db_path, b"data").await.unwrap();

        let manager = BackupManager::new(&backup_dir, 5);
        let config = json!({});

        let info = manager.create_backup(&db_path, &config).await.unwrap();
        assert!(backup_dir.join(&info.id).exists());

        manager.delete_backup(&info.id).await.unwrap();
        assert!(!backup_dir.join(&info.id).exists());

        // Cleanup
        let _ = fs::remove_dir_all(&temp_dir).await;
    }
}
