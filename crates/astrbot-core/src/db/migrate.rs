//! Database migration system using sqlx
//!
//! Provides versioned schema migrations with up/down support.

use crate::errors::{AstrBotError, Result};
use sqlx::{sqlite::SqlitePoolOptions, Pool, Sqlite};
use std::collections::HashMap;
use tracing::{error, info, warn};

/// A single migration
#[derive(Debug, Clone)]
pub struct Migration {
    pub version: i64,
    pub name: String,
    pub up_sql: String,
    pub down_sql: Option<String>,
}

/// Migration registry
#[derive(Debug, Default)]
pub struct MigrationRegistry {
    migrations: HashMap<i64, Migration>,
}

impl MigrationRegistry {
    pub fn new() -> Self {
        Self {
            migrations: HashMap::new(),
        }
    }

    /// Register a migration
    pub fn register(&mut self, migration: Migration) {
        self.migrations.insert(migration.version, migration);
    }

    /// Get all versions sorted
    pub fn versions(&self) -> Vec<i64> {
        let mut v: Vec<i64> = self.migrations.keys().copied().collect();
        v.sort();
        v
    }

    /// Get migration by version
    pub fn get(&self, version: i64) -> Option<&Migration> {
        self.migrations.get(&version)
    }

    /// Register built-in AstrBot migrations
    pub fn register_builtin(&mut self) {
        // V1: Initial schema — sessions, messages, configs
        self.register(Migration {
            version: 1,
            name: "initial_schema".to_string(),
            up_sql: r#"
CREATE TABLE IF NOT EXISTS sessions (
    id TEXT PRIMARY KEY,
    platform TEXT NOT NULL,
    session_id TEXT NOT NULL,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS messages (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id TEXT NOT NULL,
    role TEXT NOT NULL,
    content TEXT NOT NULL,
    timestamp TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (session_id) REFERENCES sessions(id)
);

CREATE TABLE IF NOT EXISTS config_store (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL,
    updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS migration_meta (
    version INTEGER PRIMARY KEY,
    name TEXT NOT NULL,
    applied_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);
            "#
            .to_string(),
            down_sql: Some(
                r#"
DROP TABLE IF EXISTS messages;
DROP TABLE IF EXISTS sessions;
DROP TABLE IF EXISTS config_store;
DROP TABLE IF EXISTS migration_meta;
            "#
                .to_string(),
            ),
        });

        // V2: Plugin metadata table
        self.register(Migration {
            version: 2,
            name: "plugin_metadata".to_string(),
            up_sql: r#"
CREATE TABLE IF NOT EXISTS plugin_metadata (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    version TEXT NOT NULL,
    enabled BOOLEAN DEFAULT 1,
    config_json TEXT,
    installed_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);
            "#
            .to_string(),
            down_sql: Some(
                r#"
DROP TABLE IF EXISTS plugin_metadata;
            "#
                .to_string(),
            ),
        });

        // V3: Safety audit log
        self.register(Migration {
            version: 3,
            name: "safety_audit_log".to_string(),
            up_sql: r#"
CREATE TABLE IF NOT EXISTS safety_audit_log (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    message_id TEXT,
    check_type TEXT NOT NULL,
    result TEXT NOT NULL,
    details TEXT,
    timestamp TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);
            "#
            .to_string(),
            down_sql: Some(
                r#"
DROP TABLE IF EXISTS safety_audit_log;
            "#
                .to_string(),
            ),
        });
    }
}

/// Migration runner
pub struct MigrationRunner {
    pool: Pool<Sqlite>,
    registry: MigrationRegistry,
}

impl MigrationRunner {
    pub fn new(pool: Pool<Sqlite>, registry: MigrationRegistry) -> Self {
        Self { pool, registry }
    }

    /// Get current schema version from DB
    pub async fn current_version(&self) -> Result<i64> {
        let result = sqlx::query_as::<_, (i64,)>("SELECT MAX(version) FROM migration_meta")
            .fetch_optional(&self.pool)
            .await;

        match result {
            Ok(row) => Ok(row.map(|r| r.0).unwrap_or(0)),
            Err(sqlx::Error::Database(db_err)) if db_err.message().contains("no such table") => {
                Ok(0)
            }
            Err(e) => Err(AstrBotError::Database(format!(
                "Migration version check: {}",
                e
            ))),
        }
    }

    /// Run all pending migrations (up)
    pub async fn migrate(&self) -> Result<Vec<i64>> {
        let current = self.current_version().await?;
        let versions = self.registry.versions();
        let mut applied = Vec::new();

        for version in versions {
            if version > current {
                let migration = self.registry.get(version).ok_or_else(|| {
                    AstrBotError::Internal(format!("Migration {} not found", version))
                })?;

                info!("Applying migration v{}: {}", version, migration.name);

                // Run migration in a transaction
                let mut tx = self.pool.begin().await.map_err(|e| {
                    AstrBotError::Database(format!("Migration transaction start: {}", e))
                })?;

                sqlx::query(&migration.up_sql)
                    .execute(&mut *tx)
                    .await
                    .map_err(|e| {
                        AstrBotError::Database(format!("Migration v{} up failed: {}", version, e))
                    })?;

                sqlx::query("INSERT INTO migration_meta (version, name) VALUES (?, ?)")
                    .bind(version)
                    .bind(&migration.name)
                    .execute(&mut *tx)
                    .await
                    .map_err(|e| {
                        AstrBotError::Database(format!("Migration v{} meta insert: {}", version, e))
                    })?;

                tx.commit().await.map_err(|e| {
                    AstrBotError::Database(format!("Migration v{} commit: {}", version, e))
                })?;

                applied.push(version);
                info!("Migration v{} applied successfully", version);
            }
        }

        Ok(applied)
    }

    /// Rollback to a specific version
    pub async fn rollback_to(&self, target_version: i64) -> Result<Vec<i64>> {
        let current = self.current_version().await?;
        let mut rolled_back = Vec::new();

        if target_version >= current {
            return Ok(rolled_back);
        }

        // Rollback in reverse order
        let mut versions = self.registry.versions();
        versions.reverse();

        for version in versions {
            if version > target_version && version <= current {
                let migration = self.registry.get(version).ok_or_else(|| {
                    AstrBotError::Internal(format!("Migration {} not found", version))
                })?;

                if let Some(down_sql) = &migration.down_sql {
                    info!("Rolling back migration v{}: {}", version, migration.name);

                    let mut tx = self.pool.begin().await.map_err(|e| {
                        AstrBotError::Database(format!("Rollback transaction start: {}", e))
                    })?;

                    sqlx::query(down_sql).execute(&mut *tx).await.map_err(|e| {
                        AstrBotError::Database(format!("Migration v{} down failed: {}", version, e))
                    })?;

                    sqlx::query("DELETE FROM migration_meta WHERE version = ?")
                        .bind(version)
                        .execute(&mut *tx)
                        .await
                        .map_err(|e| {
                            AstrBotError::Database(format!(
                                "Migration v{} meta delete: {}",
                                version, e
                            ))
                        })?;

                    tx.commit().await.map_err(|e| {
                        AstrBotError::Database(format!("Rollback v{} commit: {}", version, e))
                    })?;

                    rolled_back.push(version);
                    info!("Migration v{} rolled back successfully", version);
                } else {
                    warn!(
                        "Migration v{} has no down script — cannot rollback",
                        version
                    );
                }
            }
        }

        Ok(rolled_back)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn create_test_pool() -> Pool<Sqlite> {
        SqlitePoolOptions::new()
            .connect("sqlite::memory:")
            .await
            .expect("Failed to create test pool")
    }

    #[tokio::test]
    async fn test_migration_registry_builtin() {
        let mut registry = MigrationRegistry::new();
        registry.register_builtin();
        let versions = registry.versions();
        assert_eq!(versions, vec![1, 2, 3]);
    }

    #[tokio::test]
    async fn test_migration_runner_migrate() {
        let pool = create_test_pool().await;
        let mut registry = MigrationRegistry::new();
        registry.register_builtin();

        let runner = MigrationRunner::new(pool, registry);
        let applied = runner.migrate().await.unwrap();
        assert_eq!(applied, vec![1, 2, 3]);

        let current = runner.current_version().await.unwrap();
        assert_eq!(current, 3);
    }

    #[tokio::test]
    async fn test_migration_runner_idempotent() {
        let pool = create_test_pool().await;
        let mut registry = MigrationRegistry::new();
        registry.register_builtin();

        let runner = MigrationRunner::new(pool, registry);

        // First run
        let applied1 = runner.migrate().await.unwrap();
        assert_eq!(applied1, vec![1, 2, 3]);

        // Second run — should apply nothing
        let applied2 = runner.migrate().await.unwrap();
        assert!(applied2.is_empty());
    }

    #[tokio::test]
    async fn test_migration_runner_rollback() {
        let pool = create_test_pool().await;
        let mut registry = MigrationRegistry::new();
        registry.register_builtin();

        let runner = MigrationRunner::new(pool, registry);
        runner.migrate().await.unwrap();

        let rolled_back = runner.rollback_to(1).await.unwrap();
        assert_eq!(rolled_back, vec![3, 2]);

        let current = runner.current_version().await.unwrap();
        assert_eq!(current, 1);
    }
}
