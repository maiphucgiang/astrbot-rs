use axum::{
    Router,
    routing::{get, post, put, delete},
    extract::{Path, State, Query, WebSocketUpgrade},
    extract::ws::{WebSocket, Message as WsMessage},
    Json,
};
use astrbot_core::config::AstrBotConfig;
use astrbot_persona::{PersonaManager, CustomPersonaRequest, ReplyStyle};
use astrbot_provider::{ChatProvider, ChatMessage, ChatOptions, ProviderConfig};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Dashboard 共享状态
#[derive(Clone)]
pub struct AppState {
    /// 运行配置（结构化，可热重载持久化）
    pub config: Arc<RwLock<AstrBotConfig>>,
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
    /// 日志条目列表（内存中保留最近 1000 条）
    pub logs: Arc<RwLock<Vec<Value>>>,
    /// 系统启动时间
    pub start_time: std::time::Instant,
    /// 配置文件路径
    pub config_path: String,
}

impl AppState {
    pub fn new() -> Self {
        let persona_mgr = PersonaManager::new(Some("data/personas.db".to_string()));
        Self {
            config: Arc::new(RwLock::new(AstrBotConfig::default())),
            providers: Arc::new(RwLock::new(vec![])),
            platforms: Arc::new(RwLock::new(vec![])),
            plugins: Arc::new(RwLock::new(vec![])),
            persona_manager: Arc::new(std::sync::Mutex::new(persona_mgr)),
            sessions: Arc::new(RwLock::new(vec![])),
            logs: Arc::new(RwLock::new(vec![])),
            start_time: std::time::Instant::now(),
            config_path: "data/config.json".to_string(),
        }
    }

    /// 从配置文件加载配置
    pub async fn load_config(&self) -> anyhow::Result<()> {
        let path = &self.config_path;
        if tokio::fs::try_exists(path).await.unwrap_or(false) {
            let config = AstrBotConfig::from_file(path).await?;
            let mut cfg = self.config.write().await;
            *cfg = config;
        }
        Ok(())
    }

    /// 保存配置到文件
    pub async fn save_config(&self) -> anyhow::Result<()> {
        let cfg = self.config.read().await;
        AstrBotConfig::to_file(&*cfg, &self.config_path).await?;
        Ok(())
    }

    /// 同步内存 providers 到配置并持久化
    pub async fn sync_providers_and_save(&self) -> anyhow::Result<()> {
        let providers = self.providers.read().await;
        let mut cfg = self.config.write().await;
        cfg.providers = providers.iter()
            .filter_map(|v| serde_json::from_value(v.clone()).ok())
            .collect();
        drop(cfg);
        self.save_config().await
    }
}

/// 启动 Dashboard Web 服务
pub async fn start_server() {
    let state = AppState::new();
    if let Err(e) = state.load_config().await {
        tracing::warn!("Failed to load config: {}", e);
    }
    let app = build_router(state);

    let addr = SocketAddr::from(([0, 0, 0, 0], 6185));
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
        // WebChat WebSocket
        .route("/ws/chat", get(chat_ws_handler))
        // 静态文件（dashboard SPA）
        .fallback_service(
            tower_http::services::ServeDir::new("./dashboard/dist")
                .fallback(tower_http::services::ServeFile::new("./dashboard/dist/index.html"))
        )
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
    let config = serde_json::to_value(&*state.config.read().await).unwrap_or_default();
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
    Json(serde_json::to_value(&*state.config.read().await).unwrap_or_default())
}

async fn update_config(
    State(state): State<AppState>,
    Json(body): Json<Value>,
) -> Json<Value> {
    match serde_json::from_value::<AstrBotConfig>(body) {
        Ok(new_cfg) => {
            let mut cfg = state.config.write().await;
            *cfg = new_cfg;
            Json(json!({"success": true}))
        }
        Err(e) => Json(json!({"success": false, "error": format!("Invalid config: {}", e)})),
    }
}

async fn get_config_key(
    State(state): State<AppState>,
    Path(key): Path<String>,
) -> Json<Value> {
    let cfg = state.config.read().await;
    let cfg_val = serde_json::to_value(&*cfg).unwrap_or_default();
    Json(cfg_val.get(&key).cloned().unwrap_or(Value::Null))
}

async fn update_config_key(
    State(state): State<AppState>,
    Path(key): Path<String>,
    Json(body): Json<Value>,
) -> Json<Value> {
    let mut cfg = state.config.write().await;
    let mut cfg_val = serde_json::to_value(&*cfg).unwrap_or_default();
    if let Some(obj) = cfg_val.as_object_mut() {
        obj.insert(key, body);
    }
    match serde_json::from_value::<AstrBotConfig>(cfg_val) {
        Ok(new_cfg) => {
            *cfg = new_cfg;
            Json(json!({"success": true}))
        }
        Err(e) => Json(json!({"success": false, "error": format!("Invalid config: {}", e)})),
    }
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
    let mut providers = state.providers.write().await;
    let idx = providers.iter().position(|p| {
        p.get("id").and_then(|v| v.as_str()) == Some(&id)
            || p.get("name").and_then(|v| v.as_str()) == Some(&id)
    });

    match idx {
        Some(i) => {
            let existing = &providers[i];
            let mut updated = existing.clone();
            if let Some(obj) = updated.as_object_mut() {
                if let Some(body_obj) = body.as_object() {
                    for (k, v) in body_obj {
                        obj.insert(k.clone(), v.clone());
                    }
                }
                obj.insert("id".to_string(), json!(id));
            }
            providers[i] = updated.clone();
            Json(json!({"success": true, "provider": updated}))
        }
        None => Json(json!({"success": false, "error": "Provider not found"})),
    }
}

async fn delete_provider(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Json<Value> {
    let mut providers = state.providers.write().await;
    let len_before = providers.len();
    providers.retain(|p| {
        p.get("id").and_then(|v| v.as_str()) != Some(&id)
            && p.get("name").and_then(|v| v.as_str()) != Some(&id)
    });
    let deleted = providers.len() < len_before;
    Json(json!({"success": deleted, "deleted": id}))
}

async fn test_provider(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Json<Value> {
    let cfg = {
        let providers = state.providers.read().await;
        let provider_cfg = providers.iter().find(|p| {
            p.get("id").and_then(|v| v.as_str()) == Some(&id)
                || p.get("name").and_then(|v| v.as_str()) == Some(&id)
        });
        match provider_cfg.cloned() {
            Some(v) => v,
            None => return Json(json!({"success": false, "error": "Provider not found"})),
        }
    };

    let provider_type = cfg.get("provider_type").and_then(|v| v.as_str()).unwrap_or("openai");
    let api_key = cfg.get("api_key").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let base_url = cfg.get("base_url").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let model = cfg.get("model").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let name = cfg.get("name").and_then(|v| v.as_str()).unwrap_or(provider_type).to_string();

    let config = ProviderConfig {
        name: name.clone(),
        base_url,
        api_key,
        model: model.clone(),
        extra_headers: None,
    };

    let start = std::time::Instant::now();
    let test_msg = ChatMessage::user("Hello, this is a connectivity test from AstrBot Dashboard.");
    let options = ChatOptions::default();

    let result = _test_provider_chat(provider_type, config, id, test_msg, options).await;
    let latency_ms = start.elapsed().as_millis() as u64;

    let mut response = result;
    if let Some(obj) = response.as_object_mut() {
        obj.insert("latency_ms".to_string(), json!(latency_ms));
    }
    Json(response)
}

async fn _test_provider_chat(
    provider_type: &str,
    config: ProviderConfig,
    id: String,
    test_msg: ChatMessage,
    options: ChatOptions,
) -> Value {
    let provider: Box<dyn ChatProvider> = match provider_type {
        "openai" => Box::new(astrbot_provider::OpenAiCompatibleProvider::new(config)),
        "moonshot" => Box::new(astrbot_provider::sources::moonshot::create(config.api_key, config.model)),
        "deepseek" => Box::new(astrbot_provider::sources::deepseek::create(config.api_key, config.model)),
        "groq" => Box::new(astrbot_provider::sources::groq::create(config.api_key, config.model)),
        "openrouter" => Box::new(astrbot_provider::sources::openrouter::create(config.api_key, config.model)),
        "siliconflow" => Box::new(astrbot_provider::sources::siliconflow::create(config.api_key, config.model)),
        "zhipu" => Box::new(astrbot_provider::sources::zhipu::create(config.api_key, config.model)),
        "xai" => Box::new(astrbot_provider::sources::xai::create(config.api_key, config.model)),
        "minimax" => Box::new(astrbot_provider::sources::minimax::create(config.api_key, config.model)),
        "volcengine" => Box::new(astrbot_provider::sources::volcengine::create(config.api_key, config.model)),
        "qwen" => Box::new(astrbot_provider::sources::qwen::create(config.api_key, config.model)),
        "stepfun" => Box::new(astrbot_provider::sources::stepfun::create(config.api_key, config.model)),
        "hyperbolic" => Box::new(astrbot_provider::sources::hyperbolic::create(config.api_key, config.model)),
        "oneapi" => Box::new(astrbot_provider::sources::oneapi::create(config.base_url, config.api_key, config.model)),
        "lmstudio" => Box::new(astrbot_provider::sources::lmstudio::create(config.base_url, config.model)),
        "ai21" => Box::new(astrbot_provider::sources::ai21::create(config.api_key, config.model)),
        "azure" => Box::new(astrbot_provider::sources::azure::create(config.api_key, config.model)),
        "baichuan" => Box::new(astrbot_provider::sources::baichuan::create(config.api_key, config.model)),
        "cohere" => Box::new(astrbot_provider::sources::cohere::create(config.api_key, config.model)),
        "fireworks" => Box::new(astrbot_provider::sources::fireworks::create(config.api_key, config.model)),
        "perplexity" => Box::new(astrbot_provider::sources::perplexity::create(config.api_key, config.model)),
        "together" => Box::new(astrbot_provider::sources::together::create(config.api_key, config.model)),
        "zerooneai" => Box::new(astrbot_provider::sources::zerooneai::create(config.api_key, config.model)),
        "baidu" => Box::new(astrbot_provider::sources::baidu::create(config.api_key, config.model)),
        _ => {
            return json!({
                "success": false,
                "provider": id,
                "error": format!("Provider type '{}' not supported", provider_type),
                "message": "Provider test failed"
            });
        }
    };

    match provider.chat(vec![test_msg], options).await {
        Ok(reply) => {
            json!({
                "success": true,
                "provider": id,
                "reply_preview": reply.chars().take(100).collect::<String>(),
                "message": "Provider is online and responding"
            })
        }
        Err(e) => {
            json!({
                "success": false,
                "provider": id,
                "error": e.to_string(),
                "message": "Provider test failed"
            })
        }
    }
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
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> Json<Value> {
    let mut platforms = state.platforms.write().await;
    if let Some(idx) = platforms.iter().position(|p| {
        p.get("id").and_then(|v| v.as_str()) == Some(&id)
    }) {
        let mut updated = platforms[idx].clone();
        if let Some(obj) = updated.as_object_mut() {
            if let Some(body_obj) = body.as_object() {
                for (key, val) in body_obj.iter() {
                    obj.insert(key.clone(), val.clone());
                }
            }
            obj.insert("updated_at".to_string(), json!(format!("{:?}", std::time::SystemTime::now())));
        }
        platforms[idx] = updated.clone();
        return Json(json!({"success": true, "updated": id, "platform": updated}));
    }
    Json(json!({"success": false, "error": format!("Platform '{}' not found", id)}))
}

// ========== 插件 ==========

async fn list_plugins(State(state): State<AppState>) -> Json<Value> {
    Json(json!({"plugins": *state.plugins.read().await}))
}

async fn install_plugin(
    State(state): State<AppState>,
    Json(body): Json<Value>,
) -> Json<Value> {
    let plugin_id = body.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
    if plugin_id.is_empty() {
        return Json(json!({"success": false, "error": "Plugin id is required"}));
    }

    let mut plugins = state.plugins.write().await;
    let exists = plugins.iter().any(|p| p.get("id").and_then(|v| v.as_str()) == Some(&plugin_id));
    if exists {
        return Json(json!({"success": false, "error": format!("Plugin '{}' already installed", plugin_id)}));
    }

    let mut plugin = body.clone();
    if let Some(obj) = plugin.as_object_mut() {
        obj.entry("enabled".to_string()).or_insert(json!(true));
        obj.entry("installed_at".to_string()).or_insert(json!(format!("{:?}", std::time::SystemTime::now())));
    }
    plugins.push(plugin.clone());
    Json(json!({"success": true, "plugin": plugin}))
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
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Json<Value> {
    let mut plugins = state.plugins.write().await;
    let len_before = plugins.len();
    plugins.retain(|p| p.get("id").and_then(|v| v.as_str()) != Some(&id));
    let deleted = plugins.len() < len_before;
    Json(json!({"success": deleted, "uninstalled": id}))
}

async fn toggle_plugin(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Json<Value> {
    let mut plugins = state.plugins.write().await;
    match plugins.iter_mut().find(|p| p.get("id").and_then(|v| v.as_str()) == Some(&id)) {
        Some(plugin) => {
            if let Some(obj) = plugin.as_object_mut() {
                let current = obj.get("enabled").and_then(|v| v.as_bool()).unwrap_or(true);
                obj.insert("enabled".to_string(), json!(!current));
                return Json(json!({"success": true, "plugin_id": id, "enabled": !current}));
            }
            Json(json!({"success": false, "error": "Invalid plugin structure"}))
        }
        None => Json(json!({"success": false, "error": "Plugin not found"})),
    }
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
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Json<Value> {
    let sessions = state.sessions.read().await;
    let session = sessions.iter().find(|s| s.get("id").and_then(|v| v.as_str()) == Some(&id));
    let messages = session
        .and_then(|s| s.get("messages").cloned())
        .unwrap_or(json!([]));
    Json(json!({"session_id": id, "messages": messages}))
}

// ========== 消息历史 ==========

async fn list_history(
    State(state): State<AppState>,
    Query(params): Query<HashMap<String, String>>,
) -> Json<Value> {
    let sessions = state.sessions.read().await;
    let limit = params.get("limit").and_then(|s| s.parse::<usize>().ok()).unwrap_or(50);
    let offset = params.get("offset").and_then(|s| s.parse::<usize>().ok()).unwrap_or(0);
    let total = sessions.len();
    let paginated: Vec<Value> = sessions.iter().skip(offset).take(limit).cloned().collect();
    Json(json!({
        "sessions": paginated,
        "total": total,
        "limit": limit,
        "offset": offset,
        "has_more": offset + limit < total,
    }))
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
    Json(serde_json::to_value(&*state.config.read().await).unwrap_or_default())
}

async fn update_settings(
    State(state): State<AppState>,
    Json(body): Json<Value>,
) -> Json<Value> {
    match serde_json::from_value::<AstrBotConfig>(body) {
        Ok(new_cfg) => {
            let mut cfg = state.config.write().await;
            *cfg = new_cfg;
            Json(json!({"success": true}))
        }
        Err(e) => Json(json!({"success": false, "error": format!("Invalid config: {}", e)})),
    }
}

async fn get_setting(
    State(state): State<AppState>,
    Path(key): Path<String>,
) -> Json<Value> {
    let cfg = state.config.read().await;
    let cfg_val = serde_json::to_value(&*cfg).unwrap_or_default();
    Json(cfg_val.get(&key).cloned().unwrap_or(Value::Null))
}

async fn update_setting(
    State(state): State<AppState>,
    Path(key): Path<String>,
    Json(body): Json<Value>,
) -> Json<Value> {
    let mut cfg = state.config.write().await;
    let mut cfg_val = serde_json::to_value(&*cfg).unwrap_or_default();
    if let Some(obj) = cfg_val.as_object_mut() {
        obj.insert(key, body);
    }
    match serde_json::from_value::<AstrBotConfig>(cfg_val) {
        Ok(new_cfg) => {
            *cfg = new_cfg;
            Json(json!({"success": true}))
        }
        Err(e) => Json(json!({"success": false, "error": format!("Invalid config: {}", e)})),
    }
}

// ========== 日志 ==========

async fn get_logs(State(state): State<AppState>) -> Json<Value> {
    let logs = state.logs.read().await;
    Json(json!({"logs": logs.clone(), "count": logs.len(), "max_retained": 1000}))
}

// ========== WebChat WebSocket ==========

async fn chat_ws_handler(
    State(state): State<AppState>,
    ws: WebSocketUpgrade,
) -> axum::response::Response {
    ws.on_upgrade(|socket| handle_socket(socket, state))
}

async fn handle_socket(mut socket: WebSocket, state: AppState) {
    use astrbot_provider::ChatProvider;
    use uuid::Uuid;

    while let Some(Ok(msg)) = socket.recv().await {
        if let WsMessage::Text(text) = msg {
            let req: Value = match serde_json::from_str(&text) {
                Ok(v) => v,
                Err(_) => {
                    let _ = socket.send(WsMessage::Text(json!({
                        "error": "Invalid JSON"
                    }).to_string())).await;
                    continue;
                }
            };

            let user_message = req.get("message").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let session_id = req.get("session_id")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .unwrap_or_else(|| format!("webchat_{}", Uuid::new_v4()));

            if user_message.is_empty() {
                let _ = socket.send(WsMessage::Text(json!({
                    "error": "message is required"
                }).to_string())).await;
                continue;
            }

            // Save user message to session (in-memory)
            {
                let mut sessions = state.sessions.write().await;
                let session = sessions.iter_mut().find(|s| {
                    s.get("id").and_then(|v| v.as_str()) == Some(&session_id)
                });
                if let Some(s) = session {
                    if let Some(obj) = s.as_object_mut() {
                        let msgs = obj.entry("messages").or_insert_with(|| json!([]));
                        if let Some(arr) = msgs.as_array_mut() {
                            arr.push(json!({
                                "id": format!("msg_{}", Uuid::new_v4()),
                                "role": "user",
                                "content": user_message.clone(),
                                "created_at": chrono::Utc::now().to_rfc3339()
                            }));
                        }
                    }
                } else {
                    sessions.push(json!({
                        "id": session_id.clone(),
                        "platform": "webchat",
                        "chat_id": session_id.clone(),
                        "title": Some(format!("WebChat {}", &session_id[..8.min(session_id.len())])),
                        "created_at": chrono::Utc::now().to_rfc3339(),
                        "updated_at": chrono::Utc::now().to_rfc3339(),
                        "messages": [
                            {
                                "id": format!("msg_{}", Uuid::new_v4()),
                                "role": "user",
                                "content": user_message.clone(),
                                "created_at": chrono::Utc::now().to_rfc3339()
                            }
                        ]
                    }));
                }
            }

            // Find default provider and chat
            let provider_cfg = {
                let providers = state.providers.read().await;
                providers.iter().find(|p| {
                    p.get("enabled").and_then(|v| v.as_bool()).unwrap_or(true)
                }).cloned()
            };

            let reply = if let Some(cfg) = provider_cfg {
                let provider_type = cfg.get("provider_type").and_then(|v| v.as_str()).unwrap_or("openai");
                let api_key = cfg.get("api_key").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let base_url = cfg.get("base_url").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let model = cfg.get("model").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let name = cfg.get("name").and_then(|v| v.as_str()).unwrap_or(provider_type).to_string();

                let config = ProviderConfig {
                    name: name.clone(),
                    base_url,
                    api_key,
                    model: model.clone(),
                    extra_headers: None,
                };

                let provider_result: Result<Box<dyn ChatProvider>, String> = match provider_type {
                    "openai" => Ok(Box::new(astrbot_provider::OpenAiCompatibleProvider::new(config))),
                    "moonshot" => Ok(Box::new(astrbot_provider::sources::moonshot::create(config.api_key, config.model))),
                    "deepseek" => Ok(Box::new(astrbot_provider::sources::deepseek::create(config.api_key, config.model))),
                    "groq" => Ok(Box::new(astrbot_provider::sources::groq::create(config.api_key, config.model))),
                    "openrouter" => Ok(Box::new(astrbot_provider::sources::openrouter::create(config.api_key, config.model))),
                    "siliconflow" => Ok(Box::new(astrbot_provider::sources::siliconflow::create(config.api_key, config.model))),
                    "zhipu" => Ok(Box::new(astrbot_provider::sources::zhipu::create(config.api_key, config.model))),
                    "xai" => Ok(Box::new(astrbot_provider::sources::xai::create(config.api_key, config.model))),
                    "minimax" => Ok(Box::new(astrbot_provider::sources::minimax::create(config.api_key, config.model))),
                    "volcengine" => Ok(Box::new(astrbot_provider::sources::volcengine::create(config.api_key, config.model))),
                    "qwen" => Ok(Box::new(astrbot_provider::sources::qwen::create(config.api_key, config.model))),
                    "stepfun" => Ok(Box::new(astrbot_provider::sources::stepfun::create(config.api_key, config.model))),
                    "hyperbolic" => Ok(Box::new(astrbot_provider::sources::hyperbolic::create(config.api_key, config.model))),
                    "oneapi" => Ok(Box::new(astrbot_provider::sources::oneapi::create(config.base_url, config.api_key, config.model))),
                    "lmstudio" => Ok(Box::new(astrbot_provider::sources::lmstudio::create(config.base_url, config.model))),
                    _ => Err(format!("[Provider type '{}' not supported in WebChat]", provider_type))
                };

                match provider_result {
                    Ok(provider) => {
                        let test_msg = ChatMessage::user(&user_message);
                        let options = ChatOptions::default();
                        match provider.chat(vec![test_msg], options).await {
                            Ok(reply_text) => reply_text,
                            Err(e) => format!("[Error: {}]", e),
                        }
                    }
                    Err(err_msg) => err_msg
                }
            } else {
                "[No provider configured. Please add a provider in Dashboard.]".to_string()
            };

            // Save assistant reply to session
            {
                let mut sessions = state.sessions.write().await;
                if let Some(s) = sessions.iter_mut().find(|s| {
                    s.get("id").and_then(|v| v.as_str()) == Some(&session_id)
                }) {
                    if let Some(obj) = s.as_object_mut() {
                        let msgs = obj.entry("messages").or_insert_with(|| json!([]));
                        if let Some(arr) = msgs.as_array_mut() {
                            arr.push(json!({
                                "id": format!("msg_{}", Uuid::new_v4()),
                                "role": "assistant",
                                "content": reply.clone(),
                                "created_at": chrono::Utc::now().to_rfc3339()
                            }));
                        }
                        obj.insert("updated_at".to_string(), json!(chrono::Utc::now().to_rfc3339()));
                    }
                }
            }

            let resp = json!({
                "reply": reply,
                "session_id": session_id,
                "role": "assistant",
                "done": true
            });

            if socket.send(WsMessage::Text(resp.to_string())).await.is_err() {
                break;
            }
        }
    }
}
