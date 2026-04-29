//! Database persistence layer for AstrBot
//!
//! Provides SQLite-based storage for:
//! - Message history
//! - Sessions/conversations
//! - Users

use crate::errors::{AstrBotError, Result};
use crate::provider::ChatMessage;
use chrono::{DateTime, Utc};
use sqlx::sqlite::{SqlitePool, SqlitePoolOptions};

pub mod migrate;

/// Database manager
pub struct Database {
    pool: SqlitePool,
}

impl Database {
    /// Create a new database manager with the given SQLite URL
    pub async fn new(database_url: &str) -> Result<Self> {
        let pool = if database_url.starts_with("sqlite:")
            && !database_url.starts_with("sqlite::memory:")
        {
            let path = database_url.trim_start_matches("sqlite:");
            let options = sqlx::sqlite::SqliteConnectOptions::new()
                .filename(path)
                .create_if_missing(true);
            SqlitePoolOptions::new()
                .max_connections(5)
                .connect_with(options)
                .await
                .map_err(|e| {
                    AstrBotError::Internal(format!("Failed to connect to database: {}", e))
                })?
        } else {
            SqlitePoolOptions::new()
                .max_connections(5)
                .connect(database_url)
                .await
                .map_err(|e| {
                    AstrBotError::Internal(format!("Failed to connect to database: {}", e))
                })?
        };

        let db = Self { pool };
        db.init().await?;
        Ok(db)
    }

    /// Create a new in-memory database (for testing)
    pub async fn new_in_memory() -> Result<Self> {
        Self::new("sqlite::memory:").await
    }

    /// Initialize database tables
    async fn init(&self) -> Result<()> {
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS sessions (
                id TEXT PRIMARY KEY,
                platform TEXT NOT NULL,
                chat_id TEXT NOT NULL,
                title TEXT,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS users (
                id TEXT PRIMARY KEY,
                platform_user_id TEXT NOT NULL,
                platform TEXT NOT NULL,
                nickname TEXT,
                created_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS message_history (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                session_id TEXT NOT NULL,
                user_id TEXT,
                role TEXT NOT NULL,
                content TEXT NOT NULL,
                model TEXT,
                created_at TEXT NOT NULL,
                FOREIGN KEY (session_id) REFERENCES sessions(id)
            );

            CREATE TABLE IF NOT EXISTS personas (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                system_prompt TEXT NOT NULL,
                variables TEXT NOT NULL DEFAULT '{}',
                is_default INTEGER NOT NULL DEFAULT 0,
                description TEXT,
                created_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS active_persona (
                key TEXT PRIMARY KEY DEFAULT 'global',
                persona_id TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_messages_session ON message_history(session_id);
            CREATE INDEX IF NOT EXISTS idx_messages_created ON message_history(created_at);
            "#,
        )
        .execute(&self.pool)
        .await
        .map_err(|e| AstrBotError::Internal(format!("Failed to init database: {}", e)))?;

        Ok(())
    }

    // ============== Session Operations ==============

    /// Create a new session
    pub async fn create_session(
        &self,
        id: &str,
        platform: &str,
        chat_id: &str,
        title: Option<&str>,
    ) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO sessions (id, platform, chat_id, title, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?)"
        )
        .bind(id)
        .bind(platform)
        .bind(chat_id)
        .bind(title)
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(|e| AstrBotError::Internal(format!("Failed to create session: {}", e)))?;

        Ok(())
    }

    /// Get a session by ID
    pub async fn get_session(&self, id: &str) -> Result<Option<Session>> {
        let row = sqlx::query_as::<_, Session>(
            "SELECT id, platform, chat_id, title, created_at, updated_at FROM sessions WHERE id = ?"
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AstrBotError::Internal(format!("Failed to get session: {}", e)))?;

        Ok(row)
    }

    /// Update session updated_at
    pub async fn touch_session(&self, id: &str) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        sqlx::query("UPDATE sessions SET updated_at = ? WHERE id = ?")
            .bind(&now)
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(|e| AstrBotError::Internal(format!("Failed to touch session: {}", e)))?;

        Ok(())
    }

    /// List recent sessions
    pub async fn list_sessions(&self, limit: i64) -> Result<Vec<Session>> {
        let sessions = sqlx::query_as::<_, Session>(
            "SELECT id, platform, chat_id, title, created_at, updated_at FROM sessions ORDER BY updated_at DESC LIMIT ?"
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AstrBotError::Internal(format!("Failed to list sessions: {}", e)))?;

        Ok(sessions)
    }

    // ============== User Operations ==============

    /// Create or update a user
    pub async fn upsert_user(
        &self,
        id: &str,
        platform_user_id: &str,
        platform: &str,
        nickname: Option<&str>,
    ) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO users (id, platform_user_id, platform, nickname, created_at) VALUES (?, ?, ?, ?, ?)
             ON CONFLICT(id) DO UPDATE SET nickname = excluded.nickname"
        )
        .bind(id)
        .bind(platform_user_id)
        .bind(platform)
        .bind(nickname)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(|e| AstrBotError::Internal(format!("Failed to upsert user: {}", e)))?;

        Ok(())
    }

    /// Get a user by ID
    pub async fn get_user(&self, id: &str) -> Result<Option<User>> {
        let row = sqlx::query_as::<_, User>(
            "SELECT id, platform_user_id, platform, nickname, created_at FROM users WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AstrBotError::Internal(format!("Failed to get user: {}", e)))?;

        Ok(row)
    }

    // ============== Message History Operations ==============

    /// Save a message to history
    pub async fn save_message(
        &self,
        session_id: &str,
        user_id: Option<&str>,
        role: &str,
        content: &str,
        model: Option<&str>,
    ) -> Result<i64> {
        let now = Utc::now().to_rfc3339();
        let result = sqlx::query(
            "INSERT INTO message_history (session_id, user_id, role, content, model, created_at) VALUES (?, ?, ?, ?, ?, ?)"
        )
        .bind(session_id)
        .bind(user_id)
        .bind(role)
        .bind(content)
        .bind(model)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(|e| AstrBotError::Internal(format!("Failed to save message: {}", e)))?;

        Ok(result.last_insert_rowid())
    }

    /// Get message history for a session
    pub async fn get_session_messages(
        &self,
        session_id: &str,
        limit: i64,
    ) -> Result<Vec<MessageRecord>> {
        let messages = sqlx::query_as::<_, MessageRecord>(
            "SELECT id, session_id, user_id, role, content, model, created_at FROM message_history WHERE session_id = ? ORDER BY created_at DESC LIMIT ?"
        )
        .bind(session_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AstrBotError::Internal(format!("Failed to get messages: {}", e)))?;

        Ok(messages)
    }

    /// Get messages as ChatMessage format (for LLM context)
    pub async fn get_messages_as_chat(
        &self,
        session_id: &str,
        limit: i64,
    ) -> Result<Vec<ChatMessage>> {
        let records = self.get_session_messages(session_id, limit).await?;
        let messages: Vec<ChatMessage> = records
            .into_iter()
            .rev() // Reverse to get chronological order
            .map(|r| ChatMessage {
                role: r.role,
                content: r.content,
                name: r.user_id,
                tool_call_id: None,
                tool_calls: None,
            })
            .collect();

        Ok(messages)
    }

    /// Delete messages by session ID
    pub async fn delete_by_session_id(&self, session_id: &str) -> Result<u64> {
        let result = sqlx::query("DELETE FROM message_history WHERE session_id = ?")
            .bind(session_id)
            .execute(&self.pool)
            .await
            .map_err(|e| {
                AstrBotError::Internal(format!("Failed to delete messages by session: {}", e))
            })?;

        Ok(result.rows_affected())
    }

    /// Delete old messages
    pub async fn delete_old_messages(&self, before: DateTime<Utc>) -> Result<u64> {
        let before_str = before.to_rfc3339();
        let result = sqlx::query("DELETE FROM message_history WHERE created_at < ?")
            .bind(&before_str)
            .execute(&self.pool)
            .await
            .map_err(|e| AstrBotError::Internal(format!("Failed to delete old messages: {}", e)))?;

        Ok(result.rows_affected())
    }

    /// Count messages in a session
    pub async fn count_messages(&self, session_id: &str) -> Result<i64> {
        let count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM message_history WHERE session_id = ?")
                .bind(session_id)
                .fetch_one(&self.pool)
                .await
                .map_err(|e| AstrBotError::Internal(format!("Failed to count messages: {}", e)))?;

        Ok(count)
    }

    /// Save or update a persona
    pub async fn save_persona(
        &self,
        id: &str,
        name: &str,
        system_prompt: &str,
        variables: &str, // JSON string
        is_default: bool,
        description: Option<&str>,
    ) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        let is_default_i: i64 = if is_default { 1 } else { 0 };
        sqlx::query(
            "INSERT INTO personas (id, name, system_prompt, variables, is_default, description, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(id) DO UPDATE SET
                name = excluded.name,
                system_prompt = excluded.system_prompt,
                variables = excluded.variables,
                is_default = excluded.is_default,
                description = excluded.description"
        )
        .bind(id)
        .bind(name)
        .bind(system_prompt)
        .bind(variables)
        .bind(is_default_i)
        .bind(description)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(|e| AstrBotError::Internal(format!("Failed to save persona: {}", e)))?;

        Ok(())
    }

    /// Load all personas from database
    pub async fn load_personas(&self) -> Result<Vec<PersonaRecord>> {
        let rows = sqlx::query_as::<_, PersonaRecord>(
            "SELECT id, name, system_prompt, variables, is_default, description, created_at FROM personas ORDER BY created_at DESC"
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AstrBotError::Internal(format!("Failed to load personas: {}", e)))?;

        Ok(rows)
    }

    /// Delete a persona by ID
    pub async fn delete_persona(&self, id: &str) -> Result<()> {
        sqlx::query("DELETE FROM personas WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(|e| AstrBotError::Internal(format!("Failed to delete persona: {}", e)))?;

        Ok(())
    }

    /// Save active persona ID
    pub async fn save_active_persona(&self, id: &str) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO active_persona (key, persona_id, updated_at) VALUES ('global', ?, ?)
             ON CONFLICT(key) DO UPDATE SET persona_id = excluded.persona_id, updated_at = excluded.updated_at"
        )
        .bind(id)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(|e| AstrBotError::Internal(format!("Failed to save active persona: {}", e)))?;

        Ok(())
    }

    /// Load active persona ID
    pub async fn load_active_persona(&self) -> Result<Option<String>> {
        let row: Option<(String,)> =
            sqlx::query_as("SELECT persona_id FROM active_persona WHERE key = 'global'")
                .fetch_optional(&self.pool)
                .await
                .map_err(|e| {
                    AstrBotError::Internal(format!("Failed to load active persona: {}", e))
                })?;

        Ok(row.map(|r| r.0))
    }

    /// Get paginated message history for a session (cursor-based)
    ///
    /// Returns: (messages, next_cursor, has_more)
    /// - cursor: id of the oldest message in the previous page (None for first page)
    /// - Ordering: newest first (id DESC)
    pub async fn get_session_messages_paginated(
        &self,
        session_id: &str,
        cursor: Option<i64>,
        limit: i64,
    ) -> Result<(Vec<MessageRecord>, Option<i64>, bool)> {
        // Fetch limit + 1 to determine has_more
        let fetch_limit = limit + 1;

        let messages = match cursor {
            Some(cursor_id) => {
                sqlx::query_as::<_, MessageRecord>(
                    "SELECT id, session_id, user_id, role, content, model, created_at 
                     FROM message_history 
                     WHERE session_id = ? AND id < ? 
                     ORDER BY id DESC 
                     LIMIT ?",
                )
                .bind(session_id)
                .bind(cursor_id)
                .bind(fetch_limit)
                .fetch_all(&self.pool)
                .await
            }
            None => {
                sqlx::query_as::<_, MessageRecord>(
                    "SELECT id, session_id, user_id, role, content, model, created_at 
                     FROM message_history 
                     WHERE session_id = ? 
                     ORDER BY id DESC 
                     LIMIT ?",
                )
                .bind(session_id)
                .bind(fetch_limit)
                .fetch_all(&self.pool)
                .await
            }
        }
        .map_err(|e| AstrBotError::Internal(format!("Failed to get paginated messages: {}", e)))?;

        let has_more = messages.len() > limit as usize;
        let mut result = messages;
        if has_more {
            result.truncate(limit as usize);
        }

        let next_cursor = if has_more {
            result.last().map(|m| m.id)
        } else {
            None
        };

        Ok((result, next_cursor, has_more))
    }
}

/// Persona database record
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct PersonaRecord {
    pub id: String,
    pub name: String,
    pub system_prompt: String,
    pub variables: String, // JSON
    pub is_default: i64,
    pub description: Option<String>,
    pub created_at: String,
}

/// Session record
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Session {
    pub id: String,
    pub platform: String,
    pub chat_id: String,
    pub title: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// User record
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct User {
    pub id: String,
    pub platform_user_id: String,
    pub platform: String,
    pub nickname: Option<String>,
    pub created_at: String,
}

/// Message record
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct MessageRecord {
    pub id: i64,
    pub session_id: String,
    pub user_id: Option<String>,
    pub role: String,
    pub content: String,
    pub model: Option<String>,
    pub created_at: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_database_init() {
        let db = Database::new_in_memory().await.unwrap();
        // If init succeeds, tables exist
        let count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM sqlite_master WHERE type='table'")
                .fetch_one(&db.pool)
                .await
                .unwrap();
        assert!(count >= 3); // sessions, users, message_history
    }

    #[tokio::test]
    async fn test_session_crud() {
        let db = Database::new_in_memory().await.unwrap();

        db.create_session("sess1", "qq", "123456", Some("Test Chat"))
            .await
            .unwrap();

        let session = db.get_session("sess1").await.unwrap().unwrap();
        assert_eq!(session.platform, "qq");
        assert_eq!(session.chat_id, "123456");
        assert_eq!(session.title, Some("Test Chat".to_string()));

        db.touch_session("sess1").await.unwrap();

        let sessions = db.list_sessions(10).await.unwrap();
        assert_eq!(sessions.len(), 1);
    }

    #[tokio::test]
    async fn test_user_upsert() {
        let db = Database::new_in_memory().await.unwrap();

        db.upsert_user("u1", "qq_123", "qq", Some("Alice"))
            .await
            .unwrap();
        let user = db.get_user("u1").await.unwrap().unwrap();
        assert_eq!(user.nickname, Some("Alice".to_string()));

        // Update nickname
        db.upsert_user("u1", "qq_123", "qq", Some("Alice2"))
            .await
            .unwrap();
        let user = db.get_user("u1").await.unwrap().unwrap();
        assert_eq!(user.nickname, Some("Alice2".to_string()));
    }

    #[tokio::test]
    async fn test_message_history() {
        let db = Database::new_in_memory().await.unwrap();
        db.create_session("sess1", "qq", "123456", None)
            .await
            .unwrap();

        let msg_id = db
            .save_message("sess1", Some("u1"), "user", "Hello", Some("gpt-4"))
            .await
            .unwrap();
        assert!(msg_id > 0);

        db.save_message("sess1", None, "assistant", "Hi there", Some("gpt-4"))
            .await
            .unwrap();

        let messages = db.get_session_messages("sess1", 10).await.unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].role, "assistant"); // DESC order
        assert_eq!(messages[1].role, "user");

        let count = db.count_messages("sess1").await.unwrap();
        assert_eq!(count, 2);
    }

    #[tokio::test]
    async fn test_messages_as_chat() {
        let db = Database::new_in_memory().await.unwrap();
        db.create_session("sess1", "qq", "123456", None)
            .await
            .unwrap();

        db.save_message("sess1", Some("u1"), "user", "Hello", None)
            .await
            .unwrap();
        db.save_message("sess1", None, "assistant", "Hi", None)
            .await
            .unwrap();
        db.save_message("sess1", Some("u1"), "user", "How are you?", None)
            .await
            .unwrap();

        let chat_messages = db.get_messages_as_chat("sess1", 10).await.unwrap();
        assert_eq!(chat_messages.len(), 3);
        assert_eq!(chat_messages[0].content, "Hello");
        assert_eq!(chat_messages[1].content, "Hi");
        assert_eq!(chat_messages[2].content, "How are you?");
    }

    #[tokio::test]
    async fn test_delete_old_messages() {
        let db = Database::new_in_memory().await.unwrap();
        db.create_session("sess1", "qq", "123456", None)
            .await
            .unwrap();

        db.save_message("sess1", None, "user", "Old", None)
            .await
            .unwrap();
        db.save_message("sess1", None, "user", "New", None)
            .await
            .unwrap();

        let deleted = db.delete_old_messages(Utc::now()).await.unwrap();
        assert_eq!(deleted, 2);

        let count = db.count_messages("sess1").await.unwrap();
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn test_multiple_sessions_isolated() {
        let db = Database::new_in_memory().await.unwrap();
        db.create_session("sess1", "qq", "123", None).await.unwrap();
        db.create_session("sess2", "telegram", "456", None)
            .await
            .unwrap();

        db.save_message("sess1", None, "user", "msg1", None)
            .await
            .unwrap();
        db.save_message("sess2", None, "user", "msg2", None)
            .await
            .unwrap();

        let m1 = db.get_session_messages("sess1", 10).await.unwrap();
        let m2 = db.get_session_messages("sess2", 10).await.unwrap();
        assert_eq!(m1.len(), 1);
        assert_eq!(m2.len(), 1);
        assert_eq!(m1[0].content, "msg1");
        assert_eq!(m2[0].content, "msg2");
    }
}
