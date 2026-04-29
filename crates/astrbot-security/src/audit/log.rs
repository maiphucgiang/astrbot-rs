use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

/// Audit log entry
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AuditEntry {
    pub id: String,
    pub timestamp: DateTime<Utc>,
    pub level: AuditLevel,
    pub actor: String,    // who did it (user_id, bot_id, system)
    pub action: String,   // what happened
    pub resource: String, // what was affected
    pub details: Option<String>,
    pub ip: Option<String>,
    pub success: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum AuditLevel {
    Info,
    Warning,
    Error,
    Critical,
}

impl std::fmt::Display for AuditLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AuditLevel::Info => write!(f, "INFO"),
            AuditLevel::Warning => write!(f, "WARN"),
            AuditLevel::Error => write!(f, "ERROR"),
            AuditLevel::Critical => write!(f, "CRITICAL"),
        }
    }
}

/// In-memory audit logger with auto-rotation
pub struct AuditLogger {
    entries: Arc<Mutex<VecDeque<AuditEntry>>>,
    max_entries: usize,
}

impl AuditLogger {
    pub fn new(max_entries: usize) -> Self {
        Self {
            entries: Arc::new(Mutex::new(VecDeque::with_capacity(max_entries))),
            max_entries,
        }
    }

    pub fn default() -> Self {
        Self::new(10_000)
    }

    /// Log a new audit entry
    pub fn log(
        &self,
        level: AuditLevel,
        actor: &str,
        action: &str,
        resource: &str,
        success: bool,
        details: Option<&str>,
        ip: Option<&str>,
    ) {
        let entry = AuditEntry {
            id: uuid::Uuid::new_v4().to_string(),
            timestamp: Utc::now(),
            level,
            actor: actor.to_string(),
            action: action.to_string(),
            resource: resource.to_string(),
            details: details.map(|s| s.to_string()),
            ip: ip.map(|s| s.to_string()),
            success,
        };

        let mut entries = self.entries.lock().unwrap();
        if entries.len() >= self.max_entries {
            entries.pop_front(); // auto-rotation: remove oldest
        }
        entries.push_back(entry);
    }

    /// Convenience methods
    pub fn info(&self, actor: &str, action: &str, resource: &str) {
        self.log(AuditLevel::Info, actor, action, resource, true, None, None);
    }

    pub fn warn(&self, actor: &str, action: &str, resource: &str, details: &str) {
        self.log(
            AuditLevel::Warning,
            actor,
            action,
            resource,
            true,
            Some(details),
            None,
        );
    }

    pub fn error(&self, actor: &str, action: &str, resource: &str, details: &str) {
        self.log(
            AuditLevel::Error,
            actor,
            action,
            resource,
            false,
            Some(details),
            None,
        );
    }

    pub fn critical(&self, actor: &str, action: &str, resource: &str, details: &str, ip: &str) {
        self.log(
            AuditLevel::Critical,
            actor,
            action,
            resource,
            false,
            Some(details),
            Some(ip),
        );
    }

    /// Query entries with filters
    pub fn query(
        &self,
        level: Option<AuditLevel>,
        actor: Option<&str>,
        action: Option<&str>,
        since: Option<DateTime<Utc>>,
        until: Option<DateTime<Utc>>,
        limit: Option<usize>,
    ) -> Vec<AuditEntry> {
        let entries = self.entries.lock().unwrap();
        let limit = limit.unwrap_or(100);

        entries
            .iter()
            .rev() // newest first
            .filter(|e| {
                if let Some(ref l) = level {
                    e.level == *l
                } else {
                    true
                }
            })
            .filter(|e| {
                if let Some(a) = actor {
                    e.actor == a
                } else {
                    true
                }
            })
            .filter(|e| {
                if let Some(a) = action {
                    e.action == a
                } else {
                    true
                }
            })
            .filter(|e| {
                if let Some(s) = since {
                    e.timestamp >= s
                } else {
                    true
                }
            })
            .filter(|e| {
                if let Some(u) = until {
                    e.timestamp <= u
                } else {
                    true
                }
            })
            .take(limit)
            .cloned()
            .collect()
    }

    /// Get all entries (newest first)
    pub fn all(&self) -> Vec<AuditEntry> {
        let entries = self.entries.lock().unwrap();
        entries.iter().rev().cloned().collect()
    }

    /// Get entry count
    pub fn len(&self) -> usize {
        self.entries.lock().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Clear all entries
    pub fn clear(&self) {
        self.entries.lock().unwrap().clear();
    }

    /// Export to JSON
    pub fn export_json(&self) -> anyhow::Result<String> {
        let all = self.all();
        Ok(serde_json::to_string(&all)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_logger() -> AuditLogger {
        AuditLogger::new(100)
    }

    #[test]
    fn test_log_and_query() {
        let logger = test_logger();

        logger.info("user_1", "login", "session");
        logger.warn("user_2", "upload", "file", "large file");
        logger.error("user_1", "delete", "config", "permission denied");

        let all = logger.all();
        assert_eq!(all.len(), 3);
        assert_eq!(all[0].level, AuditLevel::Error); // newest first
        assert_eq!(all[1].level, AuditLevel::Warning);
        assert_eq!(all[2].level, AuditLevel::Info);
    }

    #[test]
    fn test_query_by_level() {
        let logger = test_logger();
        logger.info("admin", "login", "dashboard");
        logger.error("admin", "delete", "user", "fail");
        logger.info("user", "view", "page");

        let errors = logger.query(Some(AuditLevel::Error), None, None, None, None, None);
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].action, "delete");
    }

    #[test]
    fn test_query_by_actor() {
        let logger = test_logger();
        logger.info("alice", "login", "web");
        logger.info("bob", "login", "web");
        logger.info("alice", "logout", "web");

        let alice_logs = logger.query(None, Some("alice"), None, None, None, None);
        assert_eq!(alice_logs.len(), 2);
    }

    #[test]
    fn test_query_time_range() {
        let logger = test_logger();
        let now = Utc::now();

        logger.info("test", "action", "resource");

        let since = now - chrono::Duration::seconds(1);
        let until = now + chrono::Duration::seconds(1);
        let results = logger.query(None, None, None, Some(since), Some(until), None);
        assert_eq!(results.len(), 1);

        let future = now + chrono::Duration::hours(1);
        let empty = logger.query(None, None, None, Some(future), None, None);
        assert_eq!(empty.len(), 0);
    }

    #[test]
    fn test_auto_rotation() {
        let logger = AuditLogger::new(3);
        logger.info("a", "1", "r");
        logger.info("b", "2", "r");
        logger.info("c", "3", "r");
        logger.info("d", "4", "r"); // should rotate out "a"

        let all = logger.all();
        assert_eq!(all.len(), 3);
        // newest first: d, c, b
        assert_eq!(all[0].actor, "d");
        assert_eq!(all[2].actor, "b");
    }

    #[test]
    fn test_export_json() {
        let logger = test_logger();
        logger.info("sys", "start", "app");

        let json = logger.export_json().unwrap();
        assert!(json.contains("sys"));
        assert!(json.contains("start"));
    }

    #[test]
    fn test_critical_log() {
        let logger = test_logger();
        logger.critical("hacker", "sql_injection", "db", "DROP TABLE", "1.2.3.4");

        let critical = logger.query(Some(AuditLevel::Critical), None, None, None, None, None);
        assert_eq!(critical.len(), 1);
        assert_eq!(critical[0].actor, "hacker");
        assert_eq!(critical[0].ip, Some("1.2.3.4".to_string()));
        assert!(!critical[0].success);
    }
}
