use axum::{extract::State, Json};
use serde_json::{json, Value};
use std::sync::Arc;

use crate::server::AppState;
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
        Self {
            uptime_seconds: uptime,
            version: env!("CARGO_PKG_VERSION").to_string(),
            memory_used_mb: mem_used,
            memory_total_mb: mem_total,
            sse_client_count: sse_clients,
            providers_count: state.providers.read().await.len(),
            platforms_count: state.platforms.read().await.len(),
            plugins_count: state.plugins.read().await.len(),
        }
    }
}

fn get_memory_info() -> (u64, u64) {
    use sysinfo::{RefreshKind, System};
    let mut sys = System::new_with_specifics(RefreshKind::new().with_memory());
    sys.refresh_memory();
    (sys.used_memory() / 1024 / 1024, sys.total_memory() / 1024 / 1024)
}

pub async fn broadcast_config_update(state: &AppState, updated_keys: Vec<String>) {
    if let Some(ref b) = state.sse_broadcaster {
        b.broadcast_config_update(updated_keys, Some("dashboard_api".into()));
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
    Json(json!({ "status": "running", "metrics": metrics }))
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
            let persist = state.save_config().await;
            let keys: Vec<String> = body.as_object().map(|o| o.keys().cloned().collect()).unwrap_or_default();
            broadcast_config_update(&state, keys).await;
            match persist {
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
        let mut s = AppState::new();
        s.sse_broadcaster = Some(Arc::new(SseBroadcaster::new(10)));
        s
    }

    #[tokio::test]
    async fn test_metrics_from_state() {
        let s = test_state();
        let m = SystemMetrics::from_state(&s).await;
        assert!(m.uptime_seconds >= 0);
        assert_eq!(m.version, env!("CARGO_PKG_VERSION"));
        assert!(m.memory_total_mb > 0);
        assert_eq!(m.sse_client_count, 0);
    }

    #[tokio::test]
    async fn test_broadcast_config_update_reaches_client() {
        let s = test_state();
        let mut rx = s.sse_broadcaster.as_ref().unwrap().add_client().await.rx;
        broadcast_config_update(&s, vec!["providers".into(), "nickname".into()]).await;
        assert!(matches!(rx.try_recv().unwrap(), DashboardEvent::ConfigUpdate { .. }));
    }

    #[tokio::test]
    async fn test_broadcast_provider_status_reaches_client() {
        let s = test_state();
        let mut rx = s.sse_broadcaster.as_ref().unwrap().add_client().await.rx;
        broadcast_provider_status(&s, "openai", "connected", None).await;
        assert!(matches!(rx.try_recv().unwrap(), DashboardEvent::ProviderStatusChange { .. }));
    }

    #[tokio::test]
    async fn test_broadcast_plugin_change_reaches_client() {
        let s = test_state();
        let mut rx = s.sse_broadcaster.as_ref().unwrap().add_client().await.rx;
        broadcast_plugin_change(&s, "weather", "install", true).await;
        assert!(matches!(rx.try_recv().unwrap(), DashboardEvent::PluginInstall { .. }));
    }

    #[tokio::test]
    async fn test_broadcast_without_broadcaster_does_not_panic() {
        let mut s = AppState::new();
        s.sse_broadcaster = None;
        broadcast_config_update(&s, vec!["x".into()]).await;
        broadcast_provider_status(&s, "p", "ok", None).await;
        broadcast_plugin_change(&s, "pl", "install", true).await;
    }
}
