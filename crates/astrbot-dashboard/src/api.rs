//! Dashboard API handlers — enhanced status, config, and SSE broadcast integration

use axum::{
    extract::State,
    Json,
};
use serde_json::{json, Value};
use std::sync::Arc;

use crate::app_state::AppState;
use crate::sse::{DashboardEvent, SseBroadcaster};

#[derive(Debug, Clone, serde::Serialize)]
pub struct SystemMetrics {
    pub uptime_seconds: u64,
    pub version: String,
    pub memory_used_mb: u64,
    pub memory_total_mb: u64,
    pub sse_client_count: usize,
    pub providers_count: usize,
    pub platforms_count: usize,
    pub plugins_count: usize,
}

impl SystemMetrics {
    pub async fn from_state(state: &AppState) -> Self {
        let uptime = state.start_time.elapsed().as_secs();
        let (mem_used, mem_total) = get_memory_info();
        let sse_clients = match state.sse_broadcaster {
            Some(ref b) => b.client_count().await,
            None => 0,
        };
        let providers = {
            let lock = state.provider_manager.read().await;
            lock.list().len()
        };
        let platforms = {
            let cfg = state.config.read().await;
            cfg.platforms.len()
        };
        let plugins = {
            let lock = state.plugin_manager.read().await;
            lock.list().len()
        };

        Self {
            uptime_seconds: uptime,
            version: env!("CARGO_PKG_VERSION").to_string(),
            memory_used_mb: mem_used,
            memory_total_mb: mem_total,
            sse_client_count: sse_clients,
            providers_count: providers,
            platforms_count: platforms,
            plugins_count: plugins,
        }
    }
}

fn get_memory_info() -> (u64, u64) {
    use sysinfo::{System, RefreshKind};
    let mut sys = System::new_with_specifics(RefreshKind::new().with_memory(sysinfo::MemoryRefreshKind::everything()));
    sys.refresh_memory();
    let used = sys.used_memory() / 1024 / 1024;
    let total = sys.total_memory() / 1024 / 1024;
    (used, total)
}

pub async fn broadcast_config_update(state: &AppState, updated_keys: Vec<String>) {
    if let Some(ref b) = state.sse_broadcaster {
        b.broadcast_config_update(updated_keys, Some("dashboard_api".to_string()));
    }
}

pub async fn broadcast_provider_status(state: &AppState, provider_id: &str, status: &str, error: Option<String>) {
    if let Some(ref b) = state.sse_broadcaster {
        b.broadcast_provider_status(provider_id, status, error);
    }
}

pub async fn broadcast_plugin_change(state: &AppState, plugin_id: &str, action: &str, success: bool) {
    if let Some(ref b) = state.sse_broadcaster {
        b.broadcast_plugin_install(plugin_id, action, success);
    }
}

pub async fn get_enhanced_status(State(state): State<AppState>) -> Json<Value> {
    let metrics = SystemMetrics::from_state(&state).await;
    Json(json!({
        "status": "running",
        "metrics": metrics,
    }))
}

pub async fn update_config_with_broadcast(
    State(state): State<AppState>,
    Json(body): Json<Value>,
) -> Json<Value> {
    match serde_json::from_value::<astrbot_core::config::AstrBotConfig>(body.clone()) {
        Ok(new_cfg) => {
            let mut cfg = state.config.write().await;
            *cfg = new_cfg;
            drop(cfg);
            let persist_result = state.save_config().await;
            let updated_keys: Vec<String> = body
                .as_object()
                .map(|obj| obj.keys().cloned().collect())
                .unwrap_or_default();
            broadcast_config_update(&state, updated_keys).await;
            match persist_result {
                Ok(_) => Json(json!({"success": true, "persisted": true})),
                Err(e) => Json(json!({"success": true, "persisted": false, "warning": e.to_string()})),
            }
        }
        Err(e) => Json(json!({"success": false, "error": format!("Invalid config: {}", e)})),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sse::SseBroadcaster;

    fn test_state() -> AppState {
        let plugin_manager = Arc::new(tokio::sync::RwLock::new(astrbot_plugin::PluginManager::new(std::path::PathBuf::from("plugins"))));
        let provider_manager = Arc::new(tokio::sync::RwLock::new(astrbot_provider::client::ProviderManager::new()));
        let mut state = AppState::new(plugin_manager, provider_manager);
        state.sse_broadcaster = Some(Arc::new(SseBroadcaster::new(10)));
        state
    }

    #[tokio::test]
    async fn test_metrics_from_state() {
        let state = test_state();
        let m = SystemMetrics::from_state(&state).await;
        assert!(m.uptime_seconds >= 0);
        assert_eq!(m.version, env!("CARGO_PKG_VERSION"));
        assert!(m.memory_total_mb > 0);
        assert_eq!(m.sse_client_count, 0);
        assert_eq!(m.providers_count, 0);
        assert_eq!(m.platforms_count, 0);
        assert_eq!(m.plugins_count, 0);
    }

    #[tokio::test]
    async fn test_broadcast_config_update_reaches_client() {
        let state = test_state();
        let b = state.sse_broadcaster.as_ref().unwrap();
        let client = b.add_client().await;
        let mut rx = client.rx;
        broadcast_config_update(&state, vec!["providers".to_string(), "nickname".to_string()]).await;
        let received = rx.try_recv().expect("should receive event");
        assert!(matches!(received, DashboardEvent::ConfigUpdate { .. }));
    }

    #[tokio::test]
    async fn test_broadcast_provider_status_reaches_client() {
        let state = test_state();
        let b = state.sse_broadcaster.as_ref().unwrap();
        let client = b.add_client().await;
        let mut rx = client.rx;
        broadcast_provider_status(&state, "openai", "connected", None).await;
        let received = rx.try_recv().expect("should receive event");
        assert!(matches!(received, DashboardEvent::ProviderStatusChange { .. }));
    }

    #[tokio::test]
    async fn test_broadcast_plugin_change_reaches_client() {
        let state = test_state();
        let b = state.sse_broadcaster.as_ref().unwrap();
        let client = b.add_client().await;
        let mut rx = client.rx;
        broadcast_plugin_change(&state, "weather", "install", true).await;
        let received = rx.try_recv().expect("should receive event");
        assert!(matches!(received, DashboardEvent::PluginInstall { .. }));
    }

    #[tokio::test]
    async fn test_broadcast_without_broadcaster_does_not_panic() {
        let plugin_manager = Arc::new(tokio::sync::RwLock::new(astrbot_plugin::PluginManager::new(std::path::PathBuf::from("plugins"))));
        let provider_manager = Arc::new(tokio::sync::RwLock::new(astrbot_provider::client::ProviderManager::new()));
        let mut state = AppState::new(plugin_manager, provider_manager);
        state.sse_broadcaster = None;
        broadcast_config_update(&state, vec!["x".into()]).await;
        broadcast_provider_status(&state, "p", "ok", None).await;
        broadcast_plugin_change(&state, "pl", "install", true).await;
    }
}
