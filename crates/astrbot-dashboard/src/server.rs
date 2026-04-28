use axum::{
    Router,
    routing::{get, post, put, delete},
    extract::{Path, State, Query},
    Json,
};
use astrbot_persona::{PersonaManager, CustomPersonaRequest, ReplyStyle};
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
    /// 人格管理器（SQLite 持久化）
    pub persona_manager: Arc<std::sync::Mutex<PersonaManager>>,
    /// 会话列表
    pub sessions: Arc<RwLock<Vec<Value>>>,
    /// 系统启动时间
    pub start_time: std::time::Instant,
}

impl AppState {
    pub fn new() -> Self {
        let persona_mgr = PersonaManager::new(Some("data/personas.db".to_string()));
        Self {
            config: Arc::new(RwLock::new(json!({}))),
            providers: Arc::new(RwLock::new(vec![])),
            platforms: Arc::new(RwLock::new(vec![])),
            plugins: Arc::new(RwLock::new(vec![])),
            persona_manager: Arc::new(std::sync::Mutex::new(persona_mgr)),
            sessions: Arc::new(RwLock::new(vec![])),
            start_time: std::time::Instant::now(),
        }
    }
}

/// 启动 Dashboard Web 服务 (新版，使用 routes 架构 + SQLite 持久化)
pub async fn start_server() {
    let db = Arc::new(
        astrbot_core::db::Database::new_in_memory().await.unwrap()
    );

    let persona_registry = Arc::new(
        astrbot_core::persona::PersonaRegistry::with_db(db.clone())
    );
    let default_persona = astrbot_core::persona::default_persona();
    persona_registry.register(default_persona).await;
    let _ = persona_registry.switch("default").await;

    let state = crate::routes::AppState::new(env!("CARGO_PKG_VERSION").to_string())
        .with_db(db)
        .with_persona_registry(persona_registry);

    let app = crate::routes::create_router(Arc::new(state));

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
        .route("/api/personas/active", get(get_active_persona))
        .route("/api/personas/:id", get(get_persona).put(update_persona).delete(delete_persona))
        .route("/api/personas/:id/switch", post(toggle_persona))
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
    let personas = {
        let mgr = state.persona_manager.lock().unwrap();
        mgr.list_personas()
    };
    let active_persona = {
        let mgr = state.persona_manager.lock().unwrap();
        mgr.get_active_persona()
    };
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
        "active_persona": active_persona.id,
        "active_persona_name": active_persona.name,
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
    let mgr = state.persona_manager.lock().unwrap();
    let personas = mgr.list_personas();
    let persona_jsons: Vec<Value> = personas.into_iter()
        .map(|p| serde_json::to_value(p).unwrap())
        .collect();
    Json(json!({"personas": persona_jsons}))
}

async fn get_active_persona(State(state): State<AppState>) -> Json<Value> {
    let mgr = state.persona_manager.lock().unwrap();
    let persona = mgr.get_active_persona();
    Json(serde_json::to_value(persona).unwrap())
}

async fn create_persona(
    State(state): State<AppState>,
    Json(body): Json<Value>,
) -> Json<Value> {
    // Parse CustomPersonaRequest from JSON
    let req = match parse_custom_persona_request(body) {
        Ok(r) => r,
        Err(e) => return Json(json!({"success": false, "error": e})),
    };

    let mgr = state.persona_manager.lock().unwrap();
    match mgr.add_custom_persona(req) {
        Ok(persona) => Json(json!({"success": true, "created": persona})),
        Err(e) => Json(json!({"success": false, "error": e.to_string()})),
    }
}

async fn get_persona(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Json<Value> {
    let mgr = state.persona_manager.lock().unwrap();
    let personas = mgr.list_personas();
    let persona = personas.into_iter().find(|p| p.id == id);
    match persona {
        Some(p) => Json(serde_json::to_value(p).unwrap()),
        None => Json(Value::Null),
    }
}

async fn update_persona(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> Json<Value> {
    // For update, we remove the old one and add the new one
    let req = match parse_custom_persona_request(body) {
        Ok(r) => r,
        Err(e) => return Json(json!({"success": false, "error": e})),
    };

    let mgr = state.persona_manager.lock().unwrap();
    // Remove old if it's custom
    let _ = mgr.remove_persona(&id);
    match mgr.add_custom_persona(req) {
        Ok(persona) => Json(json!({"success": true, "updated": persona})),
        Err(e) => Json(json!({"success": false, "error": e.to_string()})),
    }
}

async fn delete_persona(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Json<Value> {
    let mgr = state.persona_manager.lock().unwrap();
    match mgr.remove_persona(&id) {
        Ok(()) => Json(json!({"success": true, "deleted": id})),
        Err(e) => Json(json!({"success": false, "error": e.to_string()})),
    }
}

async fn toggle_persona(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Json<Value> {
    let mgr = state.persona_manager.lock().unwrap();
    match mgr.switch_persona(&id) {
        Ok(persona) => Json(json!({
            "success": true,
            "switched_to": id,
            "persona": persona
        })),
        Err(e) => Json(json!({"success": false, "error": e.to_string()})),
    }
}

fn parse_custom_persona_request(body: Value) -> Result<CustomPersonaRequest, String> {
    let name = body.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let description = body.get("description").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let tone = body.get("tone").and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
        .unwrap_or_default();
    let catchphrases = body.get("catchphrases").and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
        .unwrap_or_default();
    let taboos = body.get("taboos").and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
        .unwrap_or_default();
    let switch_conditions = body.get("switch_conditions").and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
        .unwrap_or_default();
    let system_prompt = body.get("system_prompt").and_then(|v| v.as_str()).unwrap_or("").to_string();
    
    let reply_style = body.get("reply_style").and_then(|v| {
        Some(ReplyStyle {
            opening_pattern: v.get("opening_pattern").and_then(|s| s.as_str()).unwrap_or("").to_string(),
            sentence_length: v.get("sentence_length").and_then(|s| s.as_str()).unwrap_or("").to_string(),
            punctuation_style: v.get("punctuation_style").and_then(|s| s.as_str()).unwrap_or("").to_string(),
            emoji_usage: v.get("emoji_usage").and_then(|s| s.as_str()).unwrap_or("").to_string(),
            ending_pattern: v.get("ending_pattern").and_then(|s| s.as_str()).unwrap_or("").to_string(),
        })
    }).unwrap_or(ReplyStyle {
        opening_pattern: "".to_string(),
        sentence_length: "".to_string(),
        punctuation_style: "".to_string(),
        emoji_usage: "".to_string(),
        ending_pattern: "".to_string(),
    });

    Ok(CustomPersonaRequest {
        name, description, tone, catchphrases, taboos,
        switch_conditions, system_prompt, reply_style,
    })
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
