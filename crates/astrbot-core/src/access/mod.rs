//! Rate limiting and whitelist for AstrBot
//!
//! Provides:
//! - Token-bucket and fixed-window rate limiting per user
//! - Whitelist / blacklist for user IDs
//! - Admin bypass

use chrono::{DateTime, Duration, Utc};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::info;

/// Rate limit exceeded
#[derive(Debug, Clone)]
pub struct RateLimitExceeded {
    pub user_id: String,
    pub wait_seconds: u64,
}

/// Strategy when rate limit is hit
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RateLimitStrategy {
    /// Silently discard the message
    Discard,
    /// Stall — reply with a rate-limit message
    Stall,
    /// Queue — hold for later (not implemented)
    Queue,
}

/// Rate limit config for a single user or global
#[derive(Debug, Clone)]
pub struct RateLimitConfig {
    /// Max requests in the time window
    pub max_requests: u32,
    /// Time window in seconds
    pub window_seconds: u64,
    /// What to do when limit exceeded
    pub strategy: RateLimitStrategy,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            max_requests: 30,
            window_seconds: 60,
            strategy: RateLimitStrategy::Stall,
        }
    }
}

/// A single rate-limit window entry
#[derive(Debug)]
struct RateWindow {
    count: u32,
    reset_at: DateTime<Utc>,
}

/// Rate limiter — fixed-window per user
pub struct RateLimiter {
    config: RateLimitConfig,
    windows: HashMap<String, RateWindow>,
}

impl RateLimiter {
    pub fn new(config: RateLimitConfig) -> Self {
        Self {
            config,
            windows: HashMap::new(),
        }
    }

    /// Check if a user is allowed to send a message
    /// Returns Ok(()) if allowed, Err(RateLimitExceeded) if not
    pub fn check(&mut self, user_id: &str) -> Result<(), RateLimitExceeded> {
        let now = Utc::now();
        let window = self
            .windows
            .entry(user_id.to_string())
            .or_insert_with(|| RateWindow {
                count: 0,
                reset_at: now + Duration::seconds(self.config.window_seconds as i64),
            });

        // Reset if window expired
        if now >= window.reset_at {
            window.count = 0;
            window.reset_at = now + Duration::seconds(self.config.window_seconds as i64);
        }

        if window.count >= self.config.max_requests {
            let wait = (window.reset_at - now).num_seconds().max(0) as u64;
            return Err(RateLimitExceeded {
                user_id: user_id.to_string(),
                wait_seconds: wait,
            });
        }

        window.count += 1;
        Ok(())
    }

    /// Get current count for a user
    pub fn current_count(&mut self, user_id: &str) -> u32 {
        let now = Utc::now();
        if let Some(window) = self.windows.get_mut(user_id) {
            if now >= window.reset_at {
                0
            } else {
                window.count
            }
        } else {
            0
        }
    }

    /// Reset a user's window
    pub fn reset(&mut self, user_id: &str) {
        self.windows.remove(user_id);
    }
}

/// Access control — whitelist / blacklist / admin bypass
#[derive(Debug, Clone)]
pub struct AccessControl {
    /// Only these users are allowed (empty = allow all)
    pub whitelist: Vec<String>,
    /// These users are blocked
    pub blacklist: Vec<String>,
    /// Admin IDs (bypass all checks)
    pub admins: Vec<String>,
    /// Whether whitelist mode is active
    pub whitelist_enabled: bool,
}

impl Default for AccessControl {
    fn default() -> Self {
        Self {
            whitelist: Vec::new(),
            blacklist: Vec::new(),
            admins: Vec::new(),
            whitelist_enabled: false,
        }
    }
}

impl AccessControl {
    /// Check if a user is allowed
    pub fn is_allowed(&self, user_id: &str) -> bool {
        // Admins bypass everything
        if self.admins.contains(&user_id.to_string()) {
            return true;
        }

        // Blacklist check
        if self.blacklist.contains(&user_id.to_string()) {
            return false;
        }

        // Whitelist check
        if self.whitelist_enabled {
            return self.whitelist.contains(&user_id.to_string());
        }

        true
    }

    /// Check if user is admin
    pub fn is_admin(&self, user_id: &str) -> bool {
        self.admins.contains(&user_id.to_string())
    }

    /// Add to whitelist
    pub fn whitelist_add(&mut self, user_id: &str) {
        let id = user_id.to_string();
        if !self.whitelist.contains(&id) {
            self.whitelist.push(id);
        }
    }

    /// Remove from whitelist
    pub fn whitelist_remove(&mut self, user_id: &str) {
        self.whitelist.retain(|id| id != user_id);
    }

    /// Add to blacklist
    pub fn blacklist_add(&mut self, user_id: &str) {
        let id = user_id.to_string();
        if !self.blacklist.contains(&id) {
            self.blacklist.push(id);
        }
    }

    /// Remove from blacklist
    pub fn blacklist_remove(&mut self, user_id: &str) {
        self.blacklist.retain(|id| id != user_id);
    }

    /// Toggle whitelist mode
    pub fn set_whitelist_enabled(&mut self, enabled: bool) {
        self.whitelist_enabled = enabled;
    }
}

/// Combined rate limit + access control
pub struct AccessManager {
    pub rate_limiter: Arc<Mutex<RateLimiter>>,
    pub access_control: AccessControl,
}

impl AccessManager {
    pub fn new(rate_config: RateLimitConfig, access: AccessControl) -> Self {
        Self {
            rate_limiter: Arc::new(Mutex::new(RateLimiter::new(rate_config))),
            access_control: access,
        }
    }

    /// Check if user is allowed (access + rate)
    pub async fn check(&self, user_id: &str) -> Result<(), String> {
        // Access control first
        if !self.access_control.is_allowed(user_id) {
            return Err("Access denied".to_string());
        }

        // Rate limit (admins bypass)
        if !self.access_control.is_admin(user_id) {
            let mut limiter = self.rate_limiter.lock().await;
            if let Err(e) = limiter.check(user_id) {
                return Err(format!(
                    "Rate limit exceeded. Please wait {} seconds.",
                    e.wait_seconds
                ));
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rate_limiter_allows_under_limit() {
        let mut limiter = RateLimiter::new(RateLimitConfig {
            max_requests: 3,
            window_seconds: 60,
            strategy: RateLimitStrategy::Discard,
        });

        assert!(limiter.check("user1").is_ok());
        assert!(limiter.check("user1").is_ok());
        assert!(limiter.check("user1").is_ok());
    }

    #[test]
    fn test_rate_limiter_blocks_over_limit() {
        let mut limiter = RateLimiter::new(RateLimitConfig {
            max_requests: 2,
            window_seconds: 60,
            strategy: RateLimitStrategy::Discard,
        });

        assert!(limiter.check("user1").is_ok());
        assert!(limiter.check("user1").is_ok());
        assert!(limiter.check("user1").is_err());
    }

    #[test]
    fn test_rate_limiter_per_user() {
        let mut limiter = RateLimiter::new(RateLimitConfig {
            max_requests: 1,
            window_seconds: 60,
            strategy: RateLimitStrategy::Discard,
        });

        assert!(limiter.check("user1").is_ok());
        assert!(limiter.check("user2").is_ok()); // Different user
        assert!(limiter.check("user1").is_err()); // Same user again
    }

    #[test]
    fn test_whitelist_blocks_non_whitelisted() {
        let mut ac = AccessControl::default();
        ac.whitelist_add("alice");
        ac.set_whitelist_enabled(true);

        assert!(!ac.is_allowed("bob"));
        assert!(ac.is_allowed("alice"));
    }

    #[test]
    fn test_blacklist_blocks() {
        let mut ac = AccessControl::default();
        ac.blacklist_add("mallory");

        assert!(!ac.is_allowed("mallory"));
        assert!(ac.is_allowed("alice"));
    }

    #[test]
    fn test_admin_bypasses_all() {
        let mut ac = AccessControl::default();
        ac.whitelist_add("alice");
        ac.set_whitelist_enabled(true);
        ac.admins.push("admin1".to_string());

        assert!(ac.is_allowed("admin1")); // Admin bypasses whitelist
        assert!(!ac.is_allowed("mallory"));
    }

    #[test]
    fn test_blacklist_overrides_whitelist() {
        let mut ac = AccessControl::default();
        ac.whitelist_add("mallory");
        ac.blacklist_add("mallory");
        ac.set_whitelist_enabled(true);

        // Blacklist should still block even if whitelisted
        assert!(!ac.is_allowed("mallory"));
    }
}
