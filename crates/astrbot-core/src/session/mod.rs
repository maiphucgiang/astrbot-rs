//! Session manager — connects PlatformAdapter events with database persistence
//!
//! Manages:
//! - Session lifecycle (create/update/touch)
//! - Message history persistence
//! - Multi-turn conversation context construction

use crate::db::{Database, Session};
use crate::errors::Result;
use crate::platform::MessageSource;
use crate::provider::ChatMessage;
use std::sync::Arc;
use tracing::info;

/// Session manager — orchestrates persistence between platform events and database
pub struct SessionManager {
    db: Arc<Database>,
    /// Maximum messages to include in LLM context
    max_context_messages: usize,
    /// Whether to persist messages to database
    persist_messages: bool,
}

impl SessionManager {
    /// Create a new session manager
    pub fn new(db: Arc<Database>) -> Self {
        Self {
            db,
            max_context_messages: 20,
            persist_messages: true,
        }
    }

    /// Set max context messages
    pub fn with_max_context(mut self, max: usize) -> Self {
        self.max_context_messages = max;
        self
    }

    /// Enable/disable message persistence
    pub fn with_persistence(mut self, enabled: bool) -> Self {
        self.persist_messages = enabled;
        self
    }

    /// Generate a session ID from message source
    fn session_id(source: &MessageSource) -> String {
        format!(
            "{:?}_{}_{}",
            source.platform, source.session_id, source.user_id
        )
    }

    /// Ensure session exists in database
    pub async fn ensure_session(&self, source: &MessageSource) -> Result<String> {
        let session_id = Self::session_id(source);

        match self.db.get_session(&session_id).await? {
            Some(_) => {
                // Update last activity
                self.db.touch_session(&session_id).await?;
            }
            None => {
                info!("Creating new session: {}", session_id);
                self.db
                    .create_session(
                        &session_id,
                        &format!("{:?}", source.platform),
                        &source.session_id,
                        None,
                    )
                    .await?;
            }
        }

        // Ensure user exists
        let user_id = format!("{:?}_{}", source.platform, source.user_id);
        if self.db.get_user(&user_id).await?.is_none() {
            self.db
                .upsert_user(
                    &user_id,
                    &source.user_id,
                    &format!("{:?}", source.platform),
                    None,
                )
                .await?;
        }

        Ok(session_id)
    }

    /// Save an incoming user message
    pub async fn save_user_message(&self, source: &MessageSource, content: &str) -> Result<()> {
        if !self.persist_messages {
            return Ok(());
        }

        let session_id = self.ensure_session(source).await?;
        let user_id = format!("{:?}_{}", source.platform, source.user_id);

        self.db
            .save_message(&session_id, Some(&user_id), "user", content, None)
            .await?;

        Ok(())
    }

    /// Save an assistant response
    pub async fn save_assistant_message(
        &self,
        source: &MessageSource,
        content: &str,
        model: Option<&str>,
    ) -> Result<()> {
        if !self.persist_messages {
            return Ok(());
        }

        let session_id = Self::session_id(source);

        self.db
            .save_message(&session_id, None, "assistant", content, model)
            .await?;

        self.db.touch_session(&session_id).await?;

        Ok(())
    }

    /// Build conversation context for LLM
    pub async fn build_context(
        &self,
        source: &MessageSource,
        system_prompt: Option<&str>,
    ) -> Result<Vec<ChatMessage>> {
        let session_id = Self::session_id(source);
        let limit = self.max_context_messages as i64;

        let mut messages = self.db.get_messages_as_chat(&session_id, limit).await?;

        // Prepend system prompt if provided
        if let Some(prompt) = system_prompt {
            messages.insert(0, ChatMessage::system(prompt));
        }

        Ok(messages)
    }

    /// Get message count for a session
    pub async fn message_count(&self, source: &MessageSource) -> Result<i64> {
        let session_id = Self::session_id(source);
        self.db.count_messages(&session_id).await
    }

    /// List recent sessions
    pub async fn recent_sessions(&self, limit: i64) -> Result<Vec<Session>> {
        self.db.list_sessions(limit).await
    }

    /// Clear session history
    pub async fn clear_session(&self, source: &MessageSource) -> Result<u64> {
        let session_id = Self::session_id(source);
        self.db.delete_by_session_id(&session_id).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::{MessageSource, PlatformType};

    async fn setup_manager() -> (SessionManager, Arc<Database>) {
        let db = Arc::new(Database::new_in_memory().await.unwrap());
        let manager = SessionManager::new(db.clone());
        (manager, db)
    }

    fn test_source() -> MessageSource {
        MessageSource {
            platform: PlatformType::Aiocqhttp,
            session_id: "123456".to_string(),
            message_id: "msg_001".to_string(),
            user_id: "user_abc".to_string(),
        }
    }

    #[tokio::test]
    async fn test_ensure_session_creates_new() {
        let (manager, db) = setup_manager().await;
        let source = test_source();

        let session_id = manager.ensure_session(&source).await.unwrap();
        assert!(session_id.contains("Aiocqhttp"));
        assert!(session_id.contains("123456"));
        assert!(session_id.contains("user_abc"));

        let session = db.get_session(&session_id).await.unwrap().unwrap();
        assert_eq!(session.platform, "Aiocqhttp");
        assert_eq!(session.chat_id, "123456");
    }

    #[tokio::test]
    async fn test_save_user_and_assistant_messages() {
        let (manager, db) = setup_manager().await;
        let source = test_source();

        manager
            .save_user_message(&source, "Hello bot")
            .await
            .unwrap();
        manager
            .save_assistant_message(&source, "Hello user", Some("gpt-4"))
            .await
            .unwrap();

        let session_id = SessionManager::session_id(&source);
        let messages = db.get_session_messages(&session_id, 10).await.unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].role, "assistant");
        assert_eq!(messages[1].role, "user");
    }

    #[tokio::test]
    async fn test_build_context_with_system_prompt() {
        let (manager, _db) = setup_manager().await;
        let source = test_source();

        manager
            .save_user_message(&source, "What's 1+1?")
            .await
            .unwrap();
        manager
            .save_assistant_message(&source, "2", None)
            .await
            .unwrap();

        let context = manager
            .build_context(&source, Some("You are helpful"))
            .await
            .unwrap();
        assert_eq!(context.len(), 3);
        assert_eq!(context[0].role, "system");
        assert_eq!(context[0].content, "You are helpful");
        assert_eq!(context[1].role, "user");
        assert_eq!(context[2].role, "assistant");
    }

    #[tokio::test]
    async fn test_build_context_without_system() {
        let (manager, _db) = setup_manager().await;
        let source = test_source();

        manager.save_user_message(&source, "Hi").await.unwrap();

        let context = manager.build_context(&source, None).await.unwrap();
        assert_eq!(context.len(), 1);
        assert_eq!(context[0].role, "user");
    }

    #[tokio::test]
    async fn test_persistence_disabled() {
        let db = Arc::new(Database::new_in_memory().await.unwrap());
        let manager = SessionManager::new(db.clone()).with_persistence(false);
        let source = test_source();

        manager.save_user_message(&source, "Hello").await.unwrap();

        let session_id = SessionManager::session_id(&source);
        let count = db.count_messages(&session_id).await.unwrap();
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn test_message_count() {
        let (manager, _db) = setup_manager().await;
        let source = test_source();

        manager.save_user_message(&source, "msg1").await.unwrap();
        manager.save_user_message(&source, "msg2").await.unwrap();

        let count = manager.message_count(&source).await.unwrap();
        assert_eq!(count, 2);
    }

    #[tokio::test]
    async fn test_recent_sessions() {
        let (manager, _db) = setup_manager().await;
        let source1 = test_source();

        let mut source2 = test_source();
        source2.session_id = "789".to_string();
        source2.user_id = "user_def".to_string();

        manager.save_user_message(&source1, "hi").await.unwrap();
        manager.save_user_message(&source2, "hello").await.unwrap();

        let sessions = manager.recent_sessions(10).await.unwrap();
        assert_eq!(sessions.len(), 2);
    }

    #[tokio::test]
    async fn test_clear_session() {
        let (manager, db) = setup_manager().await;
        let source = test_source();

        manager.save_user_message(&source, "msg1").await.unwrap();
        manager.save_user_message(&source, "msg2").await.unwrap();

        let deleted = manager.clear_session(&source).await.unwrap();
        assert_eq!(deleted, 2);

        let session_id = SessionManager::session_id(&source);
        let count = db.count_messages(&session_id).await.unwrap();
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn test_session_isolation() {
        let (manager, _db) = setup_manager().await;
        let source1 = test_source();

        let mut source2 = test_source();
        source2.session_id = "999".to_string();

        manager.save_user_message(&source1, "A").await.unwrap();
        manager.save_user_message(&source2, "B").await.unwrap();

        let ctx1 = manager.build_context(&source1, None).await.unwrap();
        let ctx2 = manager.build_context(&source2, None).await.unwrap();

        assert_eq!(ctx1.len(), 1);
        assert_eq!(ctx1[0].content, "A");
        assert_eq!(ctx2.len(), 1);
        assert_eq!(ctx2[0].content, "B");
    }
}
