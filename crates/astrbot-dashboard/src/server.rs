use axum::{
    Router,
    routing::{get, post, put, delete},
    extract::{Path, State, Query},
    Json,
};
use serde_json::{json, Value};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Dashboard 共享状态
#[derive(Clone)]
pub struct AppState {
    /// 运行配置（可热重载）
    pub config: Arc<RwLock<Value>>,
    /// 提供商列表
    pub providers: Arc<RwLock<Vec<Value>>>,
    /// 平台适配器列表
    pub platforms: Arc<RwLock<Vec<Value>>>,
    /// 插件列表
    pub plugins: Arc<RwLock<Vec<Value>>>,
    /// 人格预设列表
    pub personas: Arc<RwLock<Vec<Value>>>,
    /// 会话列表
    pub sessions: Arc<RwLock<Vec<Value>>>,
    /// 系统启动时间
    pub start_time: std::time::Instant,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            config: Arc::new(RwLock::new(json!({}))),
            providers: Arc::new(RwLock::new(vec![])),
            platforms: Arc::new(RwLock::new(vec![])),
            plugins: Arc::new(RwLock::new(vec![])),
            personas: Arc::new(RwLock::new(vec![])),
            sessions: Arc::new(RwLock::new(vec![])),
            start_time: std::time::Instant::now(),
        }
    }
}

/// 启动 Dashboard Web 服务
pub async fn start_server() {
    let state = AppState::new();
    let app = build_router(state);

    let addr = SocketAddr::from(([127, 0, 0, 1], 6185));
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    tracing::info!("Dashboard listening on http://{}", addr);
    axum::serve(listener, app).await.unwrap();
}

/// 构建 API 路由
fn build_router(state: AppState) -> Router {
    Router::new()
        // 健康检查
        .route("/api/health", get(|| async { Json(json!({"status": "ok"})) }))
        // 系统状态
        .route("/api/status", get(get_status))
        .route("/api/status/detailed", get(get_detailed_status))
        // 配置
        .route("/api/config", get(get_config).put(update_config))
        .route("/api/config/:key", get(get_config_key).put(update_config_key))
        // 提供商
        .route("/api/providers", get(list_providers))
        .route("/api/providers/:id", get(get_provider).put(update_provider).delete(delete_provider))
        .route("/api/providers/:id/test", post(test_provider))
        // 平台适配器
        .route("/api/platforms", get(list_platforms))
        .route("/api/platforms/:id", get(get_platform).put(update_platform))
        // 插件
        .route("/api/plugins", get(list_plugins).post(install_plugin))
        .route("/api/plugins/:id", get(get_plugin).delete(uninstall_plugin))
        .route("/api/plugins/:id/toggle", post(toggle_plugin))
        // 人格预设
        .route("/api/personas", get(list_personas).post(create_persona))
        .route("/api/personas/:id", get(get_persona).put(update_persona).delete(delete_persona))
        .route("/api/personas/:id/toggle", post(toggle_persona))
        // 会话
        .route("/api/sessions", get(list_sessions).delete(delete_all_sessions))
        .route("/api/sessions/:id", get(get_session).delete(delete_session))
        .route("/api/sessions/:id/history", get(get_session_history))
        // 消息历史
        .route("/api/history", get(list_history))
        .route("/api/history/:id", get(get_message).delete(delete_message))
        // 设置
        .route("/api/settings", get(list_settings).put(update_settings))
        .route("/api/settings/:key", get(get_setting).put(update_setting))
        // 日志
        .route("/api/logs", get(get_logs))
        // 静态文件（dashboard SPA）
        .fallback_service(tower_http::services::ServeDir::new("./dashboard/dist"))
        .with_state(state)
}

// ========== 系统状态 ==========

async fn get_status(State(state): State<AppState>) -> Json<Value> {
    let uptime = state.start_time.elapsed().as_secs();
    Json(json!({
        "status": "running",
        "uptime_seconds": uptime,
        "version": env!("CARGO_PKG_VERSION"),
    }))
}

async fn get_detailed_status(State(state): State<AppState>) -> Json<Value> {
    let config = state.config.read().await.clone();
    let providers = state.providers.read().await.clone();
    let platforms = state.platforms.read().await.clone();
    let plugins = state.plugins.read().await.clone();
    let personas = state.personas.read().await.clone();
    let uptime = state.start_time.elapsed().as_secs();

    Json(json!({
        "status": "running",
        "uptime_seconds": uptime,
        "version": env!("CARGO_PKG_VERSION"),
        "config_summary": config,
        "providers_count": providers.len(),
        "platforms_count": platforms.len(),
        "plugins_count": plugins.len(),
        "personas_count": personas.len(),
    }))
}

// ========== 配置 ==========

async fn get_config(State(state): State<AppState>) -> Json<Value> {
    Json(state.config.read().await.clone())
}

async fn update_config(
    State(state): State<AppState>,
    Json(body): Json<Value>,
) -> Json<Value> {
    let mut cfg = state.config.write().await;
    *cfg = body;
    Json(json!({"success": true}))
}

async fn get_config_key(
    State(state): State<AppState>,
    Path(key): Path<String>,
) -> Json<Value> {
    let cfg = state.config.read().await;
    Json(cfg.get(&key).cloned().unwrap_or(Value::Null))
}

async fn update_config_key(
    State(state): State<AppState>,
    Path(key): Path<String>,
    Json(body): Json<Value>,
) -> Json<Value> {
    let mut cfg = state.config.write().await;
    if let Some(obj) = cfg.as_object_mut() {
        obj.insert(key, body);
    }
    Json(json!({"success": true}))
}

// ========== 提供商 ==========

async fn list_providers(State(state): State<AppState>) -> Json<Value> {
    Json(json!({"providers": *state.providers.read().await}))
}

async fn get_provider(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Json<Value> {
    let providers = state.providers.read().await;
    let provider = providers.iter().find(|p| {
        p.get("id").and_then(|v| v.as_str()) == Some(&id)
            || p.get("name").and_then(|v| v.as_str()) == Some(&id)
    });
    Json(provider.cloned().unwrap_or(Value::Null))
}

async fn update_provider(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> Json<Value> {
    // TODO: 接入 ProviderRegistry 更新配置
    Json(json!({"success": true, "id": id, "body": body}))
}

async fn delete_provider(
    State(_state): State<AppState>,
    Path(id): Path<String>,
) -> Json<Value> {
    // TODO: 从配置中移除 provider
    Json(json!({"success": true, "deleted": id}))
}

async fn test_provider(
    State(_state): State<AppState>,
    Path(id): Path<String>,
) -> Json<Value> {
    // TODO: 实际调用 provider.chat() 测试连通性
    Json(json!({"success": true, "provider": id, "latency_ms": 0}))
}

// ========== 平台适配器 ==========

async fn list_platforms(State(state): State<AppState>) -> Json<Value> {
    Json(json!({"platforms": *state.platforms.read().await}))
}

async fn get_platform(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Json<Value> {
    let platforms = state.platforms.read().await;
    let platform = platforms.iter().find(|p| {
        p.get("id").and_then(|v| v.as_str()) == Some(&id)
    });
    Json(platform.cloned().unwrap_or(Value::Null))
}

async fn update_platform(
    State(_state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> Json<Value> {
    // TODO: 接入 PlatformRegistry 更新配置
    Json(json!({"success": true, "id": id, "body": body}))
}

// ========== 插件 ==========

async fn list_plugins(State(state): State<AppState>) -> Json<Value> {
    Json(json!({"plugins": *state.plugins.read().await}))
}

async fn install_plugin(
    State(_state): State<AppState>,
    Json(body): Json<Value>,
) -> Json<Value> {
    // TODO: 接入 PluginManager 安装插件
    Json(json!({"success": true, "installed": body}))
}

async fn get_plugin(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Json<Value> {
    let plugins = state.plugins.read().await;
    let plugin = plugins.iter().find(|p| {
        p.get("id").and_then(|v| v.as_str()) == Some(&id)
    });
    Json(plugin.cloned().unwrap_or(Value::Null))
}

async fn uninstall_plugin(
    State(_state): State<AppState>,
    Path(id): Path<String>,
) -> Json<Value> {
    // TODO: 接入 PluginManager 卸载插件
    Json(json!({"success": true, "uninstalled": id}))
}

async fn toggle_plugin(
    State(_state): State<AppState>,
    Path(id): Path<String>,
) -> Json<Value> {
    // TODO: 启用/禁用插件
    Json(json!({"success": true, "toggled": id}))
}

// ========== 人格预设 ==========

async fn list_personas(State(state): State<AppState>) -> Json<Value> {
    Json(json!({"personas": *state.personas.read().await}))
}

async fn create_persona(
    State(state): State<AppState>,
    Json(body): Json<Value>,
) -> Json<Value> {
    state.personas.write().await.push(body.clone());
    Json(json!({"success": true, "created": body}))
}

async fn get_persona(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Json<Value> {
    let personas = state.personas.read().await;
    let persona = personas.iter().find(|p| {
        p.get("id").and_then(|v| v.as_str()) == Some(&id)
    });
    Json(persona.cloned().unwrap_or(Value::Null))
}

async fn update_persona(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> Json<Value> {
    let mut personas = state.personas.write().await;
    if let Some(idx) = personas.iter().position(|p| {
        p.get("id").and_then(|v| v.as_str()) == Some(&id)
    }) {
        personas[idx] = body;
    }
    Json(json!({"success": true, "updated": id}))
}

async fn delete_persona(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Json<Value> {
    let mut personas = state.personas.write().await;
    personas.retain(|p| p.get("id").and_then(|v| v.as_str()) != Some(&id));
    Json(json!({"success": true, "deleted": id}))
}

async fn toggle_persona(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Json<Value> {
    let mut personas = state.personas.write().await;
    if let Some(p) = personas.iter_mut().find(|p| {
        p.get("id").and_then(|v| v.as_str()) == Some(&id)
    }) {
        if let Some(obj) = p.as_object_mut() {
            let current = obj.get("active").and_then(|v| v.as_bool()).unwrap_or(false);
            obj.insert("active".to_string(), json!(!current));
        }
    }
    Json(json!({"success": true, "toggled": id}))
}

// ========== 会话 ==========

async fn list_sessions(State(state): State<AppState>) -> Json<Value> {
    Json(json!({"sessions": *state.sessions.read().await}))
}

async fn delete_all_sessions(State(state): State<AppState>) -> Json<Value> {
    state.sessions.write().await.clear();
    Json(json!({"success": true}))
}

async fn get_session(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Json<Value> {
    let sessions = state.sessions.read().await;
    let session = sessions.iter().find(|s| {
        s.get("id").and_then(|v| v.as_str()) == Some(&id)
    });
    Json(session.cloned().unwrap_or(Value::Null))
}

async fn delete_session(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Json<Value> {
    let mut sessions = state.sessions.write().await;
    sessions.retain(|s| s.get("id").and_then(|v| v.as_str()) != Some(&id));
    Json(json!({"success": true, "deleted": id}))
}

async fn get_session_history(
    State(_state): State<AppState>,
    Path(id): Path<String>,
) -> Json<Value> {
    // TODO: 接入 SessionStore 查询历史消息
    Json(json!({"session_id": id, "messages": []}))
}

// ========== 消息历史 ==========

async fn list_history(State(_state): State<AppState>) -> Json<Value> {
    // TODO: 分页查询历史消息
    Json(json!({"messages": [], "total": 0}))
}

async fn get_message(
    State(_state): State<AppState>,
    Path(_id): Path<String>,
) -> Json<Value> {
    Json(Value::Null)
}

async fn delete_message(
    State(_state): State<AppState>,
    Path(_id): Path<String>,
) -> Json<Value> {
    Json(json!({"success": true}))
}

// ========== 设置 ==========

async fn list_settings(State(state): State<AppState>) -> Json<Value> {
    Json(state.config.read().await.clone())
}

async fn update_settings(
    State(state): State<AppState>,
    Json(body): Json<Value>,
) -> Json<Value> {
    let mut cfg = state.config.write().await;
    *cfg = body;
    Json(json!({"success": true}))
}

async fn get_setting(
    State(state): State<AppState>,
    Path(key): Path<String>,
) -> Json<Value> {
    let cfg = state.config.read().await;
    Json(cfg.get(&key).cloned().unwrap_or(Value::Null))
}

async fn update_setting(
    State(state): State<AppState>,
    Path(key): Path<String>,
    Json(body): Json<Value>,
) -> Json<Value> {
    let mut cfg = state.config.write().await;
    if let Some(obj) = cfg.as_object_mut() {
        obj.insert(key, body);
    }
    Json(json!({"success": true}))
}

// ========== 日志 ==========

async fn get_logs() -> Json<Value> {
    // TODO: 接入日志系统返回最近 N 条
    Json(json!({"logs": []}))
}
