use serde::{Deserialize, Serialize};
use std::fmt;

/// Errors that can occur within AstrBot
#[derive(Debug, thiserror::Error)]
pub enum AstrBotError {
    /// Configuration error
    #[error("configuration error: {0}")]
    Config(String),
    /// Platform adapter error
    #[error("platform error [{adapter}]: {message}")]
    Platform { adapter: String, message: String },
    /// Provider (LLM) error
    #[error("provider error [{provider}]: {message}")]
    Provider { provider: String, message: String },
    /// Plugin error
    #[error("plugin error [{plugin}]: {message}")]
    Plugin { plugin: String, message: String },
    /// Database error
    #[error("database error: {0}")]
    Database(String),
    /// Network / HTTP error
    #[error("network error: {0}")]
    Network(String),
    /// Serialization error
    #[error("serialization error: {0}")]
    Serialization(String),
    /// Authentication error
    #[error("auth error: {0}")]
    Auth(String),
    /// Permission denied
    #[error("permission denied: {0}")]
    Permission(String),
    /// Generic internal error
    #[error("internal error: {0}")]
    Internal(String),
    /// Not found
    #[error("not found: {0}")]
    NotFound(String),
    /// Not implemented
    #[error("not implemented: {0}")]
    NotImplemented(String),
    /// Validation error
    #[error("validation error: {0}")]
    Validation(String),
}

/// Result type alias for AstrBot operations
pub type Result<T> = std::result::Result<T, AstrBotError>;

/// Event processing result
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventResult {
    /// Event was handled successfully
    Handled,
    /// Event was not handled (no matching handler)
    Unhandled,
    /// Event was blocked / filtered
    Blocked { reason: String },
    /// Event processing failed
    Error { message: String },
}

impl fmt::Display for EventResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EventResult::Handled => write!(f, "handled"),
            EventResult::Unhandled => write!(f, "unhandled"),
            EventResult::Blocked { reason } => write!(f, "blocked: {}", reason),
            EventResult::Error { message } => write!(f, "error: {}", message),
        }
    }
}
