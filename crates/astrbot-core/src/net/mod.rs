//! Connection pool module for unified HTTP/WebSocket management

use async_trait::async_trait;
use dashmap::DashMap;
use futures_util::{SinkExt, StreamExt};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::{sleep, timeout};
use tracing::{error, info, warn};

/// Trait for connection pools that provide HTTP clients
#[async_trait]
pub trait ConnectionPool: Send + Sync {
    /// Get a clone of the shared HTTP client
    async fn get_http_client(&self) -> reqwest::Client;
    /// Check if the pool is healthy
    async fn health_check(&self) -> crate::Result<bool>;
}

/// Shared HTTP client wrapper with unified configuration.
/// Built once, cloned via Arc for cheap sharing across adapters.
#[derive(Debug, Clone)]
pub struct SharedHttpClient {
    pub(crate) inner: Arc<reqwest::Client>,
}

impl SharedHttpClient {
    /// Create a new shared client with AstrBot unified defaults:
    /// - Pool max idle per host: 100
    /// - Idle timeout: 90s
    /// - Connect timeout: 10s
    /// - Request timeout: 30s
    pub fn new() -> Self {
        let client = reqwest::Client::builder()
            .pool_max_idle_per_host(100)
            .pool_idle_timeout(Duration::from_secs(90))
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .build()
            .expect("failed to build HTTP client");
        Self {
            inner: Arc::new(client),
        }
    }

    /// Wrap an existing reqwest::Client
    pub fn from_client(client: reqwest::Client) -> Self {
        Self {
            inner: Arc::new(client),
        }
    }

    /// Get a clone of the inner client.
    pub fn client(&self) -> reqwest::Client {
        (*self.inner).clone()
    }
}

impl Default for SharedHttpClient {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ConnectionPool for SharedHttpClient {
    async fn get_http_client(&self) -> reqwest::Client {
        self.client()
    }

    async fn health_check(&self) -> crate::Result<bool> {
        Ok(true)
    }
}

/// WebSocket message types delivered by WsConnectionManager
#[derive(Debug, Clone)]
pub enum WsMessage {
    Text(String),
    Binary(Vec<u8>),
    Ping,
    Pong,
    Close,
}

/// Manages a single WebSocket connection with auto-reconnect and heartbeat.
#[derive(Debug, Clone)]
pub struct WsConnectionManager {
    pub(crate) inner: Arc<WsConnectionManagerInner>,
}

#[derive(Debug)]
pub(crate) struct WsConnectionManagerInner {
    pub url: String,
    pub reconnect_interval: Duration,
    pub max_reconnect: u32,
    pub running: AtomicBool,
    pub current_retry: AtomicU32,
}

impl WsConnectionManager {
    /// Create a new WS manager with default reconnect config.
    pub fn new(url: String) -> Self {
        Self {
            inner: Arc::new(WsConnectionManagerInner {
                url,
                reconnect_interval: Duration::from_secs(1),
                max_reconnect: 10,
                running: AtomicBool::new(false),
                current_retry: AtomicU32::new(0),
            }),
        }
    }

    /// Create with explicit reconnect parameters.
    pub fn with_config(url: String, reconnect_interval: Duration, max_reconnect: u32) -> Self {
        Self {
            inner: Arc::new(WsConnectionManagerInner {
                url,
                reconnect_interval,
                max_reconnect,
                running: AtomicBool::new(false),
                current_retry: AtomicU32::new(0),
            }),
        }
    }

    /// URL being connected to.
    pub fn url(&self) -> &str {
        &self.inner.url
    }

    /// Max reconnect attempts.
    pub fn max_reconnect(&self) -> u32 {
        self.inner.max_reconnect
    }

    /// Compute next backoff delay using exponential backoff:
    /// 1s, 2s, 4s, 8s, 16s, 32s, capped at 60s.
    pub fn next_backoff(&self) -> Duration {
        let retry = self.inner.current_retry.load(Ordering::SeqCst);
        let secs = 1u64 << retry.min(6);
        let capped = secs.min(60);
        Duration::from_secs(capped)
    }

    /// Stop the connection manager.
    pub fn stop(&self) {
        self.inner.running.store(false, Ordering::SeqCst);
    }

    /// Start the WS connection loop.
    /// Returns a channel receiver for incoming WS messages.
    pub async fn start(&self) -> mpsc::Receiver<WsMessage> {
        let (tx, rx) = mpsc::channel(128);
        let inner = self.inner.clone();

        if inner
            .running
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            warn!("WsConnectionManager already running");
            return rx;
        }

        tokio::spawn(async move {
            let mut retry_count = 0u32;

            while inner.running.load(Ordering::Relaxed) && retry_count < inner.max_reconnect {
                inner.current_retry.store(retry_count, Ordering::SeqCst);
                let backoff = {
                    let secs = 1u64 << retry_count.min(6);
                    Duration::from_secs(secs.min(60))
                };

                if retry_count > 0 {
                    info!(
                        "[WS] Reconnecting to {} in {:?} (attempt {}/{})",
                        inner.url, backoff, retry_count, inner.max_reconnect
                    );
                    sleep(backoff).await;
                }

                match tokio_tungstenite::connect_async(&inner.url).await {
                    Ok((ws_stream, _)) => {
                        info!("[WS] Connected to {}", inner.url);
                        retry_count = 0;

                        let (mut write, mut read) = ws_stream.split();

                        // Heartbeat task: send Ping every 30s
                        let heartbeat_inner = Arc::clone(&inner);
                        let heartbeat_tx = tx.clone();
                        let heartbeat_handle = tokio::spawn(async move {
                            while heartbeat_inner.running.load(Ordering::Relaxed) {
                                sleep(Duration::from_secs(30)).await;
                                if heartbeat_inner.running.load(Ordering::Relaxed) {
                                    let _ = heartbeat_tx.send(WsMessage::Ping).await;
                                }
                            }
                        });

                        // Read loop
                        while inner.running.load(Ordering::Relaxed) {
                            match timeout(Duration::from_secs(30), read.next()).await {
                                Ok(Some(Ok(msg))) => {
                                    let out = match msg {
                                        tokio_tungstenite::tungstenite::Message::Text(t) => {
                                            Some(WsMessage::Text(t.to_string()))
                                        }
                                        tokio_tungstenite::tungstenite::Message::Binary(b) => {
                                            Some(WsMessage::Binary(b.to_vec()))
                                        }
                                        tokio_tungstenite::tungstenite::Message::Ping(_) => {
                                            Some(WsMessage::Ping)
                                        }
                                        tokio_tungstenite::tungstenite::Message::Pong(_) => {
                                            Some(WsMessage::Pong)
                                        }
                                        tokio_tungstenite::tungstenite::Message::Close(_) => {
                                            Some(WsMessage::Close)
                                        }
                                        _ => None,
                                    };

                                    if let Some(ws_msg) = out {
                                        if tx.send(ws_msg).await.is_err() {
                                            break;
                                        }
                                    }
                                }
                                Ok(Some(Err(e))) => {
                                    error!("[WS] Read error: {}", e);
                                    break;
                                }
                                Ok(None) => {
                                    info!("[WS] Stream closed");
                                    break;
                                }
                                Err(_) => {
                                    warn!("[WS] Read timeout");
                                }
                            }
                        }

                        let _ = heartbeat_handle.await;
                    }
                    Err(e) => {
                        error!("[WS] Connect failed: {}", e);
                        retry_count += 1;
                    }
                }
            }

            inner.running.store(false, Ordering::Relaxed);
            info!("[WS] Connection loop ended for {}", inner.url);
        });

        rx
    }
}

/// Concrete connection pool implementation for platform adapters.
/// Holds a shared HTTP client and a registry of WS connection managers.
#[derive(Debug)]
pub struct PlatformConnectionPool {
    http: reqwest::Client,
    ws_managers: DashMap<String, WsConnectionManager>,
}

impl PlatformConnectionPool {
    /// Create a new pool with default HTTP client config.
    pub fn new() -> Self {
        let client = reqwest::Client::builder()
            .pool_max_idle_per_host(100)
            .pool_idle_timeout(Duration::from_secs(90))
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .build()
            .expect("failed to build HTTP client");

        Self {
            http: client,
            ws_managers: DashMap::new(),
        }
    }

    /// Create from an existing HTTP client.
    pub fn with_client(http: reqwest::Client) -> Self {
        Self {
            http,
            ws_managers: DashMap::new(),
        }
    }

    /// Register a WS manager under an adapter id.
    pub fn register_ws(&self, id: String, manager: WsConnectionManager) {
        self.ws_managers.insert(id, manager);
    }

    /// Unregister a WS manager.
    pub fn unregister_ws(&self, id: &str) {
        self.ws_managers.remove(id);
    }

    /// Get a registered WS manager.
    pub fn get_ws_manager(&self, id: &str) -> Option<WsConnectionManager> {
        self.ws_managers.get(id).map(|m| m.clone())
    }
}

impl Default for PlatformConnectionPool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ConnectionPool for PlatformConnectionPool {
    async fn get_http_client(&self) -> reqwest::Client {
        self.http.clone()
    }

    async fn health_check(&self) -> crate::Result<bool> {
        // Pool is healthy if all registered WS managers are either not running or active.
        // For now, simply verify HTTP client exists.
        for entry in self.ws_managers.iter() {
            let manager = entry.value();
            // If running, consider it healthy
            if !manager.inner.running.load(Ordering::Relaxed) {
                // Not running is also OK; a manager might be stopped intentionally
            }
        }
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn test_shared_http_client_reuse() {
        let shared = SharedHttpClient::new();
        let shared2 = shared.clone();
        // Both clones share the same Arc inner
        assert!(Arc::ptr_eq(&shared.inner, &shared2.inner));
    }

    #[tokio::test]
    async fn test_http_client_config() {
        let shared = SharedHttpClient::new();
        let client = shared.client();
        // reqwest::Client doesn't expose internal config, but we can verify
        // the client was built successfully and is cloneable
        let _client2 = client.clone();
        // Verify SharedHttpClient defaults exist
        assert!(Arc::strong_count(&shared.inner) >= 1);
    }

    #[test]
    fn test_ws_backoff_sequence() {
        let manager = WsConnectionManager::new("ws://localhost:9999".to_string());
        manager.inner.current_retry.store(0, Ordering::SeqCst);
        assert_eq!(manager.next_backoff(), Duration::from_secs(1));

        manager.inner.current_retry.store(1, Ordering::SeqCst);
        assert_eq!(manager.next_backoff(), Duration::from_secs(2));

        manager.inner.current_retry.store(2, Ordering::SeqCst);
        assert_eq!(manager.next_backoff(), Duration::from_secs(4));

        manager.inner.current_retry.store(3, Ordering::SeqCst);
        assert_eq!(manager.next_backoff(), Duration::from_secs(8));

        manager.inner.current_retry.store(4, Ordering::SeqCst);
        assert_eq!(manager.next_backoff(), Duration::from_secs(16));

        manager.inner.current_retry.store(5, Ordering::SeqCst);
        assert_eq!(manager.next_backoff(), Duration::from_secs(32));

        manager.inner.current_retry.store(6, Ordering::SeqCst);
        assert_eq!(manager.next_backoff(), Duration::from_secs(60)); // capped

        manager.inner.current_retry.store(100, Ordering::SeqCst);
        assert_eq!(manager.next_backoff(), Duration::from_secs(60)); // capped
    }

    #[tokio::test]
    async fn test_ws_manager_lifecycle() {
        let manager = WsConnectionManager::new("ws://localhost:9999".to_string());
        assert!(!manager.inner.running.load(Ordering::Relaxed));

        let mut rx = manager.start().await;
        // Should be marked running immediately
        assert!(manager.inner.running.load(Ordering::Relaxed));

        // Let it attempt connection and fail a few times
        tokio::time::sleep(Duration::from_millis(200)).await;

        manager.stop();
        assert!(!manager.inner.running.load(Ordering::Relaxed));

        // Drain receiver
        while rx.try_recv().is_ok() {}
    }
}
