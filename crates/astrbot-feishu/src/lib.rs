//! AstrBot Feishu (Lark) Integration Crate
//!
//! Provides:
//! - Platform adapter for Feishu IM (messages, groups, events)
//! - Knowledge base RAG data sources (docs, bitables)
//! - Calendar event reminders
//! - Group message search/retrieval
//!
//! ## Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────┐
//! │         astrbot-feishu                  │
//! ├─────────────┬─────────────┬─────────────┤
//! │  platform   │  knowledge  │   calendar  │
//! │  (adapter)  │  (RAG src)  │  (events)   │
//! ├─────────────┴─────────────┴─────────────┤
//! │           auth + models (shared)        │
//! └─────────────────────────────────────────┘
//! ```

pub mod auth;
pub mod calendar;
pub mod knowledge;
pub mod models;
pub mod platform;
pub mod search;

// Re-export core types for convenience
pub use auth::FeishuAuth;
pub use calendar::{CalendarClient, ReminderConfig};
pub use models::CalendarEventData;
pub use knowledge::{BitableClient, DocClient, KnowledgeSource};
pub use models::*;
pub use platform::{FeishuAdapter, FeishuAdapterConfig, MessageHandler};
pub use search::{GroupMessageSearch, SearchQuery};

use thiserror::Error;

/// Top-level error type for the feishu crate
#[derive(Error, Debug)]
pub enum FeishuError {
    #[error("HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),

    #[error("API error {code}: {msg}")]
    Api { code: i32, msg: String },

    #[error("Authentication failed: {0}")]
    Auth(String),

    #[error("Serialization failed: {0}")]
    Serde(#[from] serde_json::Error),

    #[error("Invalid configuration: {0}")]
    Config(String),

    #[error("Rate limited, retry after {retry_after}s")]
    RateLimited { retry_after: u64 },

    #[error("Webhook verification failed")]
    WebhookVerify,

    #[error("Unknown error: {0}")]
    Unknown(String),
}

pub type Result<T> = std::result::Result<T, FeishuError>;

/// Initialize the crate (load env, setup tracing, etc.)
pub fn init() {
    let _ = dotenvy::dotenv();
    tracing::info!("astrbot-feishu initialized");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_display() {
        let err = FeishuError::Config("missing app_id".into());
        assert_eq!(err.to_string(), "Invalid configuration: missing app_id");
    }
}
