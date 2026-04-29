//! Webhook / Event Push system for AstrBot
//!
//! Dispatches events to registered HTTP webhooks with optional HMAC-SHA256 signatures.

use dashmap::DashMap;
use hmac::{Hmac, Mac};
use reqwest::header::{HeaderMap, HeaderValue, CONTENT_TYPE};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::Sha256;
use tracing::{debug, error, warn};

use crate::errors::{AstrBotError, Result};

type HmacSha256 = Hmac<Sha256>;

/// Configuration for a single webhook endpoint
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WebhookConfig {
    /// Unique identifier for this webhook
    pub id: String,
    /// Target URL to POST events to
    pub url: String,
    /// Event types to subscribe to (empty = all events)
    pub event_types: Vec<String>,
    /// Optional secret for HMAC-SHA256 signature
    pub secret: Option<String>,
    /// Whether this webhook is active
    pub enabled: bool,
    /// Number of retry attempts on failure (default: 3)
    #[serde(default = "default_retry_count")]
    pub retry_count: u32,
}

fn default_retry_count() -> u32 {
    3
}

impl Default for WebhookConfig {
    fn default() -> Self {
        Self {
            id: String::new(),
            url: String::new(),
            event_types: Vec::new(),
            secret: None,
            enabled: true,
            retry_count: 3,
        }
    }
}

/// Result of dispatching to a single webhook
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DispatchResult {
    /// The webhook ID that was dispatched to
    pub hook_id: String,
    /// Whether the dispatch was successful
    pub success: bool,
    /// HTTP status code if a response was received
    pub status_code: Option<u16>,
    /// Error message if dispatch failed
    pub error: Option<String>,
}

/// Manages registered webhooks and dispatches events to them
#[derive(Debug)]
pub struct WebhookManager {
    /// Registered webhooks keyed by their ID
    hooks: DashMap<String, WebhookConfig>,
    /// HTTP client for making webhook requests
    client: reqwest::Client,
}

impl Default for WebhookManager {
    fn default() -> Self {
        Self::new()
    }
}

impl WebhookManager {
    /// Create a new webhook manager
    pub fn new() -> Self {
        Self {
            hooks: DashMap::new(),
            client: reqwest::Client::new(),
        }
    }

    /// Register a new webhook configuration
    pub fn register(&self, config: WebhookConfig) -> Result<()> {
        if config.id.is_empty() {
            return Err(AstrBotError::Validation(
                "webhook id cannot be empty".to_string(),
            ));
        }
        if config.url.is_empty() {
            return Err(AstrBotError::Validation(
                "webhook url cannot be empty".to_string(),
            ));
        }

        // Validate URL is parseable
        let _ = reqwest::Url::parse(&config.url)
            .map_err(|e| AstrBotError::Validation(format!("invalid webhook url: {}", e)))?;

        let id = config.id.clone();
        self.hooks.insert(id.clone(), config);
        debug!("registered webhook {}", id);
        Ok(())
    }

    /// Unregister a webhook by ID. Returns true if it existed.
    pub fn unregister(&self, id: &str) -> bool {
        let removed = self.hooks.remove(id).is_some();
        if removed {
            debug!("unregistered webhook {}", id);
        }
        removed
    }

    /// List all registered webhooks
    pub fn list(&self) -> Vec<WebhookConfig> {
        self.hooks
            .iter()
            .map(|entry| entry.value().clone())
            .collect()
    }

    /// Dispatch an event to all matching enabled webhooks
    pub async fn dispatch(&self, event_type: &str, payload: Value) -> Vec<DispatchResult> {
        let payload_str = match serde_json::to_string(&payload) {
            Ok(s) => s,
            Err(e) => {
                error!("failed to serialize webhook payload: {}", e);
                return vec![DispatchResult {
                    hook_id: "__internal__".to_string(),
                    success: false,
                    status_code: None,
                    error: Some(format!("serialization error: {}", e)),
                }];
            }
        };

        let mut results = Vec::new();

        for entry in self.hooks.iter() {
            let hook = entry.value();

            // Skip disabled hooks
            if !hook.enabled {
                debug!("skipping disabled webhook {}", hook.id);
                continue;
            }

            // Skip hooks that filter by event type (empty = all events)
            if !hook.event_types.is_empty() && !hook.event_types.contains(&event_type.to_string()) {
                debug!(
                    "skipping webhook {} — event_type '{}' not in {:?}",
                    hook.id, event_type, hook.event_types
                );
                continue;
            }

            let result = self.dispatch_single(hook, event_type, &payload_str).await;
            results.push(result);
        }

        results
    }

    /// Dispatch to a single webhook with retry logic
    async fn dispatch_single(
        &self,
        hook: &WebhookConfig,
        event_type: &str,
        payload_str: &str,
    ) -> DispatchResult {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        headers.insert(
            "X-Event-Type",
            HeaderValue::from_str(event_type)
                .unwrap_or_else(|_| HeaderValue::from_static("unknown")),
        );

        // Add HMAC signature if secret is configured
        if let Some(secret) = &hook.secret {
            let signature = sign_payload(secret, payload_str);
            if let Ok(sig_header) = HeaderValue::from_str(&format!("sha256={}", signature)) {
                headers.insert("X-Webhook-Signature", sig_header);
            }
        }

        let mut last_error: Option<String> = None;
        let mut last_status: Option<u16> = None;
        let max_attempts = hook.retry_count.max(1);

        for attempt in 1..=max_attempts {
            match self
                .client
                .post(&hook.url)
                .headers(headers.clone())
                .body(payload_str.to_string())
                .send()
                .await
            {
                Ok(response) => {
                    last_status = Some(response.status().as_u16());
                    if response.status().is_success() {
                        return DispatchResult {
                            hook_id: hook.id.clone(),
                            success: true,
                            status_code: last_status,
                            error: None,
                        };
                    } else {
                        let status = response.status();
                        let body = response.text().await.unwrap_or_default();
                        last_error = Some(format!("HTTP {}: {}", status, body));
                        warn!(
                            "webhook {} attempt {}/{} failed: {}",
                            hook.id,
                            attempt,
                            max_attempts,
                            last_error.as_ref().unwrap()
                        );
                    }
                }
                Err(e) => {
                    last_error = Some(format!("network error: {}", e));
                    last_status = None;
                    warn!(
                        "webhook {} attempt {}/{} network error: {}",
                        hook.id, attempt, max_attempts, e
                    );
                }
            }
        }

        DispatchResult {
            hook_id: hook.id.clone(),
            success: false,
            status_code: last_status,
            error: last_error,
        }
    }

    /// Get a single webhook by ID
    pub fn get(&self, id: &str) -> Option<WebhookConfig> {
        self.hooks.get(id).map(|entry| entry.value().clone())
    }

    /// Update an existing webhook configuration
    pub fn update(&self, config: WebhookConfig) -> Result<bool> {
        if config.id.is_empty() {
            return Err(AstrBotError::Validation(
                "webhook id cannot be empty".to_string(),
            ));
        }

        if self.hooks.contains_key(&config.id) {
            self.hooks.insert(config.id.clone(), config);
            Ok(true)
        } else {
            Ok(false)
        }
    }
}

/// Generate HMAC-SHA256 hex signature for a payload
fn sign_payload(secret: &str, payload: &str) -> String {
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC key length valid");
    mac.update(payload.as_bytes());
    hex::encode(mac.finalize().into_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_register_unregisters_and_list() {
        let manager = WebhookManager::new();

        // Initially empty
        assert!(manager.list().is_empty());

        // Register a webhook
        let config = WebhookConfig {
            id: "hook-1".to_string(),
            url: "https://example.com/webhook".to_string(),
            event_types: vec!["message".to_string(), "command".to_string()],
            secret: Some("secret123".to_string()),
            enabled: true,
            retry_count: 3,
        };
        manager.register(config.clone()).unwrap();
        assert_eq!(manager.list().len(), 1);
        assert_eq!(manager.get("hook-1"), Some(config.clone()));

        // Register another
        let config2 = WebhookConfig {
            id: "hook-2".to_string(),
            url: "https://example.com/other".to_string(),
            event_types: vec![],
            secret: None,
            enabled: true,
            retry_count: 1,
        };
        manager.register(config2).unwrap();
        assert_eq!(manager.list().len(), 2);

        // Unregister
        assert!(manager.unregister("hook-1"));
        assert_eq!(manager.list().len(), 1);
        assert!(!manager.unregister("hook-1"));

        // Remaining hook
        assert_eq!(manager.list()[0].id, "hook-2");
    }

    #[test]
    fn test_register_validation_errors() {
        let manager = WebhookManager::new();

        // Empty ID
        let bad = WebhookConfig {
            id: "".to_string(),
            url: "https://example.com".to_string(),
            ..Default::default()
        };
        assert!(manager.register(bad).is_err());

        // Empty URL
        let bad2 = WebhookConfig {
            id: "x".to_string(),
            url: "".to_string(),
            ..Default::default()
        };
        assert!(manager.register(bad2).is_err());

        // Invalid URL
        let bad3 = WebhookConfig {
            id: "x".to_string(),
            url: "://not-a-url".to_string(),
            ..Default::default()
        };
        assert!(manager.register(bad3).is_err());
    }

    #[tokio::test]
    async fn test_dispatch_matching_event_type() {
        // Use httpbin as a test endpoint (returns 200 for any POST)
        let manager = WebhookManager::new();
        let config = WebhookConfig {
            id: "hook-msg".to_string(),
            url: "https://httpbin.org/post".to_string(),
            event_types: vec!["message".to_string()],
            secret: None,
            enabled: true,
            retry_count: 2,
        };
        manager.register(config).unwrap();

        let results = manager.dispatch("message", json!({"text": "hello"})).await;
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].hook_id, "hook-msg");
        // httpbin should return 200
        assert!(
            results[0].success,
            "expected success, got: {:?}",
            results[0].error
        );
        assert_eq!(results[0].status_code, Some(200));
    }

    #[tokio::test]
    async fn test_dispatch_non_matching_event_type() {
        let manager = WebhookManager::new();
        let config = WebhookConfig {
            id: "hook-msg".to_string(),
            url: "https://httpbin.org/post".to_string(),
            event_types: vec!["message".to_string()],
            secret: None,
            enabled: true,
            retry_count: 1,
        };
        manager.register(config).unwrap();

        // Dispatch a "command" event — should not match
        let results = manager.dispatch("command", json!({"cmd": "/help"})).await;
        assert!(
            results.is_empty(),
            "non-matching event should produce no dispatches"
        );
    }

    #[tokio::test]
    async fn test_dispatch_disabled_hook_skipped() {
        let manager = WebhookManager::new();
        let config = WebhookConfig {
            id: "hook-off".to_string(),
            url: "https://httpbin.org/post".to_string(),
            event_types: vec![],
            secret: None,
            enabled: false,
            retry_count: 1,
        };
        manager.register(config).unwrap();

        let results = manager.dispatch("any_event", json!({})).await;
        assert!(results.is_empty(), "disabled hook should be skipped");
    }

    #[test]
    fn test_hmac_signature_generation() {
        let secret = "my-secret";
        let payload = r#"{"event":"test","data":123}"#;
        let sig1 = sign_payload(secret, payload);
        let sig2 = sign_payload(secret, payload);

        // Deterministic
        assert_eq!(sig1, sig2);
        assert_eq!(sig1.len(), 64); // 32 bytes hex-encoded

        // Different payload => different signature
        let sig3 = sign_payload(secret, r#"{"event":"other"}"#);
        assert_ne!(sig1, sig3);

        // Different secret => different signature
        let sig4 = sign_payload("other-secret", payload);
        assert_ne!(sig1, sig4);
    }

    #[tokio::test]
    async fn test_dispatch_with_hmac_signature() {
        let manager = WebhookManager::new();
        let config = WebhookConfig {
            id: "hook-secure".to_string(),
            url: "https://httpbin.org/post".to_string(),
            event_types: vec![],
            secret: Some("super-secret".to_string()),
            enabled: true,
            retry_count: 2,
        };
        manager.register(config).unwrap();

        let payload = json!({"action": "ping"});
        let results = manager.dispatch("notice", payload.clone()).await;
        assert_eq!(results.len(), 1);
        assert!(
            results[0].success,
            "httpbin should accept POST: {:?}",
            results[0].error
        );
    }

    #[tokio::test]
    async fn test_dispatch_empty_event_types_catches_all() {
        let manager = WebhookManager::new();
        let config = WebhookConfig {
            id: "hook-all".to_string(),
            url: "https://httpbin.org/post".to_string(),
            event_types: vec![], // empty = all events
            secret: None,
            enabled: true,
            retry_count: 1,
        };
        manager.register(config).unwrap();

        let results = manager.dispatch("whatever", json!({})).await;
        assert_eq!(results.len(), 1);
        assert!(results[0].success);
    }

    #[test]
    fn test_update_webhook() {
        let manager = WebhookManager::new();
        let config = WebhookConfig {
            id: "hook-up".to_string(),
            url: "https://old.com".to_string(),
            ..Default::default()
        };
        manager.register(config).unwrap();

        let updated = WebhookConfig {
            id: "hook-up".to_string(),
            url: "https://new.com".to_string(),
            enabled: false,
            ..Default::default()
        };
        assert!(manager.update(updated.clone()).unwrap());
        assert_eq!(manager.get("hook-up").unwrap().url, "https://new.com");
        assert!(!manager.get("hook-up").unwrap().enabled);

        // Updating non-existent hook returns false
        assert!(!manager
            .update(WebhookConfig {
                id: "nope".to_string(),
                ..Default::default()
            })
            .unwrap());
    }

    #[tokio::test]
    async fn test_dispatch_retry_on_failure() {
        let manager = WebhookManager::new();
        // Use a URL that will definitely fail (invalid port on localhost)
        let config = WebhookConfig {
            id: "hook-fail".to_string(),
            url: "http://127.0.0.1:1/webhook".to_string(),
            event_types: vec![],
            secret: None,
            enabled: true,
            retry_count: 2,
        };
        manager.register(config).unwrap();

        let results = manager.dispatch("test", json!({})).await;
        assert_eq!(results.len(), 1);
        assert!(!results[0].success);
        assert!(results[0].error.is_some());
        // Should have attempted 2 times
    }
}
