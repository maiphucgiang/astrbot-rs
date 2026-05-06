use axum::{
    Router,
    routing::{get, post, put, delete},
    extract::{Path, State, Query, WebSocketUpgrade},
    extract::ws::{WebSocket, Message as WsMessage},
    Json,
};
use crate::api::{get_enhanced_status, update_config_with_broadcast, broadcast_config_update, broadcast_provider_status, broadcast_plugin_change};
use crate::app_state::AppState;
use crate::sse::SseBroadcaster;
use astrbot_core::config::AstrBotConfig;
use astrbot_persona::{PersonaManager, CustomPersonaRequest, ReplyStyle};
use astrbot_provider::{ChatProvider, ChatMessage, ChatOptions, ProviderConfig};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::RwLock;

pub async fn start_server() {
    let state = AppState::new(env!("CARGO_PKG_VERSION"));
    if let Err(e) = state.load_config().await {
        tracing::warn!("Failed to load config: {}", e);
    }
    let app = build_router(state);
    let addr = SocketAddr::from(([0, 0, 0, 0], 6185));
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    tracing::info!("Dashboard listening on http://{}", addr);
    axum::serve(listener, app).await.unwrap();
}

fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/api/health", get(|| async { Json(json!({"status": "ok"})) }))
        .route("/api/status", get(get_enhanced_status))
        .route("/api/status/detailed", get(get_detailed_status))
        .route("/api/config", get(get_config).put(update_config_with_broadcast))
        .route("/api/config/:key", get(get_config_key).put(update_config_key))
        .route("/api/providers", get(list_providers))
        .route("/api/providers/:id", get(get_provider).put(update_provider).delete(delete_provider))
        .route("/api/providers/:id/test", post(test_provider))
        .route("/api/platforms", get(list_platforms))
        .route("/api/platforms/:id", get(get_platform).put(update_platform))
        .route("/api/plugins", get(list_plugins).post(install_plugin))
        .route("/api/plugins/:id", get(get_plugin).delete(uninstall_plugin))
        .route("/api/plugins/:id/toggle", post(toggle_plugin))
        .route("/api/personas", get(list_personas).post(create_persona))
        .route("/api/personas/active", get(get_active_persona))
        .route("/api/personas/:id", get(get_persona).put(update_persona).delete(delete_persona))
        .route("/api/personas/:id/switch", post(toggle_persona))
        .route("/api/sessions", get(list_sessions).delete(delete_all_sessions))
        .route("/api/sessions/:id", get(get_session).delete(delete_session))
        .route("/api/sessions/:id/history", get(get_session_history))
        .route("/api/history", get(list_history))
        .route("/api/history/:id", get(get_message).delete(delete_message))
        .route("/api/settings", get(list_settings).put(update_settings))
        .route("/api/settings/:key", get(get_setting).put(update_setting))
        .route("/api/logs", get(get_logs))
        .route("/api/events", get(crate::sse::events_handler))
        .route("/ws/chat", get(chat_ws_handler))
        .fallback_service(
            tower_http::services::ServeDir::new("./dashboard/dist")
                .fallback(tower_http::services::ServeFile::new("./dashboard/dist/index.html"))
        )
        .with_state(state)
}

async fn get_status(State(state): State<AppState>) -> Json<Value> {
    let uptime = state.start_time.elapsed().as_secs();
    Json(json!({"status": "running", "uptime_seconds": uptime, "version": env!("CARGO_PKG_VERSION")}))
}

async fn get_detailed_status(State(state): State<AppState>) -> Json<Value> {
    let config = serde_json::to_value(&*state.config.read().await).unwrap_or_default();
    let providers_count = if let Some(ref pvm) = state.provider_manager {
        let lock = pvm.read().await; lock.list().len()
    } else { 0 };
    let platforms_count = { let cfg = state.config.read().await; cfg.platforms.len() };
    let plugins_count = if let Some(ref pm) = state.plugin_manager {
        let lock = pm.read().await; lock.list().len()
    } else { 0 };
    let personas = { let mgr = state.persona_manager.lock().unwrap(); mgr.list_personas() };
    let active_persona = { let mgr = state.persona_manager.lock().unwrap(); mgr.get_active_persona() };
    let uptime = state.start_time.elapsed().as_secs();
    Json(json!({
        "status": "running", "uptime_seconds": uptime, "version": env!("CARGO_PKG_VERSION"),
        "config_summary": config, "providers_count": providers_count,
        "platforms_count": platforms_count, "plugins_count": plugins_count,
        "personas_count": personas.len(), "active_persona": active_persona.id,
        "active_persona_name": active_persona.name,
    }))
}

async fn get_config(State(state): State<AppState>) -> Json<Value> {
    Json(serde_json::to_value(&*state.config.read().await).unwrap_or_default())
}

async fn update_config(State(state): State<AppState>, Json(body): Json<Value>) -> Json<Value> {
    match serde_json::from_value::<AstrBotConfig>(body) {
        Ok(new_cfg) => { let mut cfg = state.config.write().await; *cfg = new_cfg; Json(json!({"success": true})) }
        Err(e) => Json(json!({"success": false, "error": format!("Invalid config: {}", e)})),
    }
}

async fn get_config_key(State(state): State<AppState>, Path(key): Path<String>) -> Json<Value> {
    let cfg = state.config.read().await;
    let cfg_val = serde_json::to_value(&*cfg).unwrap_or_default();
    Json(cfg_val.get(&key).cloned().unwrap_or(Value::Null))
}

async fn update_config_key(State(state): State<AppState>, Path(key): Path<String>, Json(body): Json<Value>) -> Json<Value> {
    let mut cfg = state.config.write().await;
    let mut cfg_val = serde_json::to_value(&*cfg).unwrap_or_default();
    if let Some(obj) = cfg_val.as_object_mut() { obj.insert(key.clone(), body); }
    match serde_json::from_value::<AstrBotConfig>(cfg_val) {
        Ok(new_cfg) => { *cfg = new_cfg; broadcast_config_update(&state, vec![key]).await; Json(json!({"success": true})) }
        Err(e) => Json(json!({"success": false, "error": format!("Invalid config: {}", e)})),
    }
}

async fn list_providers(State(state): State<AppState>) -> Json<Value> {
    let cfg = state.config.read().await;
    let providers: Vec<Value> = cfg.providers.iter().map(|p| serde_json::to_value(p).unwrap_or(Value::Null)).collect();
    Json(json!({"providers": providers}))
}

async fn get_provider(State(state): State<AppState>, Path(id): Path<String>) -> Json<Value> {
    let cfg = state.config.read().await;
    let providers_val = serde_json::to_value(&cfg.providers).unwrap_or(json!([]));
    let provider = providers_val.as_array().and_then(|arr| {
        arr.iter().find(|p| p.get("id").and_then(|v| v.as_str()) == Some(&id) || p.get("name").and_then(|v| v.as_str()) == Some(&id))
    });
    Json(provider.cloned().unwrap_or(Value::Null))
}

async fn update_provider(State(state): State<AppState>, Path(id): Path<String>, Json(body): Json<Value>) -> Json<Value> {
    let mut cfg = state.config.write().await;
    let providers_val = serde_json::to_value(&cfg.providers).unwrap_or(json!([]));
    let mut providers = providers_val.as_array().cloned().unwrap_or_default();
    let idx = providers.iter().position(|p| {
        p.get("id").and_then(|v| v.as_str()) == Some(&id) || p.get("name").and_then(|v| v.as_str()) == Some(&id)
    });
    match idx {
        Some(i) => {
            let mut updated = providers[i].clone();
            if let Some(obj) = updated.as_object_mut() {
                if let Some(body_obj) = body.as_object() { for (k, v) in body_obj { obj.insert(k.clone(), v.clone()); } }
                obj.insert("id".to_string(), json!(id));
            }
            providers[i] = updated.clone();
            cfg.providers = providers.iter().filter_map(|v| serde_json::from_value(v.clone()).ok()).collect();
            broadcast_provider_status(&state, &id, "updated", None).await;
            Json(json!({"success": true, "provider": updated}))
        }
        None => Json(json!({"success": false, "error": "Provider not found"})),
    }
}

async fn delete_provider(State(state): State<AppState>, Path(id): Path<String>) -> Json<Value> {
    let mut cfg = state.config.write().await;
    let providers_val = serde_json::to_value(&cfg.providers).unwrap_or(json!([]));
    let mut providers = providers_val.as_array().cloned().unwrap_or_default();
    let len_before = providers.len();
    providers.retain(|p| p.get("id").and_then(|v| v.as_str()) != Some(&id) && p.get("name").and_then(|v| v.as_str()) != Some(&id));
    let deleted = providers.len() < len_before;
    cfg.providers = providers.iter().filter_map(|v| serde_json::from_value(v.clone()).ok()).collect();
    Json(json!({"success": deleted, "deleted": id}))
}

async fn test_provider(State(state): State<AppState>, Path(id): Path<String>) -> Json<Value> {
    let cfg = {
        let config = state.config.read().await;
        let providers_val = serde_json::to_value(&config.providers).unwrap_or(json!([]));
        let provider_cfg = providers_val.as_array().and_then(|arr| {
            arr.iter().find(|p| p.get("id").and_then(|v| v.as_str()) == Some(&id) || p.get("name").and_then(|v| v.as_str()) == Some(&id))
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
    let config = ProviderConfig { name: name.clone(), base_url, api_key, model: model.clone(), extra_headers: None };
    let start = std::time::Instant::now();
    let test_msg = ChatMessage::user("Hello, this is a connectivity test from AstrBot Dashboard.");
    let options = ChatOptions::default();
    let result = _test_provider_chat(provider_type, config, id, test_msg, options).await;
    let latency_ms = start.elapsed().as_millis() as u64;
    let mut response = result;
    if let Some(obj) = response.as_object_mut() { obj.insert("latency_ms".to_string(), json!(latency_ms)); }
    Json(response)
}

async fn _test_provider_chat(provider_type: &str, config: ProviderConfig, id: String, test_msg: ChatMessage, options: ChatOptions) -> Value {
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
        _ => return json!({"success": false, "provider": id, "error": format!("Provider type '{}' not supported", provider_type), "message": "Provider test failed"}),
    };
    match provider.chat(vec![test_msg], options).await {
        Ok(reply) => json!({"success": true, "provider": id, "reply_preview": reply.chars().take(100).collect::<String>(), "message": "Provider is online and responding"}),
        Err(e) => json!({"success": false, "provider": id, "error": e.to_string(), "message": "Provider test failed"}),
    }
}

async fn list_platforms(State(state): State<AppState>) -> Json<Value> {
    let cfg = state.config.read().await;
    let platforms: Vec<Value> = cfg.platforms.iter().map(|p| serde_json::to_value(p).unwrap_or(Value::Null)).collect();
    Json(json!({"platforms": platforms}))
}

async fn get_platform(State(state): State<AppState>, Path(id): Path<String>) -> Json<Value> {
    let cfg = state.config.read().await;
    let platforms_val = serde_json::to_value(&cfg.platforms).unwrap_or(json!([]));
    let platform = platforms_val.as_array().and_then(|arr| {
        arr.iter().find(|p| p.get("id").and_then(|v| v.as_str()) == Some(&id))
    });
    Json(platform.cloned().unwrap_or(Value::Null))
}

async fn update_platform(State(state): State<AppState>, Path(id): Path<String>, Json(body): Json<Value>) -> Json<Value> {
    let mut cfg = state.config.write().await;
    let platforms_val = serde_json::to_value(&cfg.platforms).unwrap_or(json!([]));
    let mut platforms = platforms_val.as_array().cloned().unwrap_or_default();
    if let Some(idx) = platforms.iter().position(|p| p.get("id").and_then(|v| v.as_str()) == Some(&id)) {
        let mut updated = platforms[idx].clone();
        if let Some(obj) = updated.as_object_mut() {
            if let Some(body_obj) = body.as_object() { for (key, val) in body_obj.iter() { obj.insert(key.clone(), val.clone()); } }
            obj.insert("updated_at".to_string(), json!(format!("{:?}", std::time::SystemTime::now())));
        }
        platforms[idx] = updated.clone();
        cfg.platforms = platforms.iter().filter_map(|v| serde_json::from_value(v.clone()).ok()).collect();
        return Json(json!({"success": true, "updated": id, "platform": updated}));
    }
    Json(json!({"success": false, "error": format!("Platform '{}' not found", id)}))
}

async fn list_plugins(State(state): State<AppState>) -> Json<Value> {
    if let Some(ref pm) = state.plugin_manager {
        let lock = pm.read().await;
        let plugins = lock.list();
        let items: Vec<Value> = plugins.iter().map(|p| {
            let plugin_cmds: Vec<String> = vec![];
            json!({
                "id": p.name, "name": p.name,
                "version": p.version, "description": "",
                "author": "", "enabled": p.activated,
                "status": if p.activated { "loaded" } else { "disabled" },
                "commands": plugin_cmds,
            })
        }).collect();
        let enabled_count = plugins.iter().filter(|p| p.activated).count();
        return Json(json!({"plugins": items, "total": items.len(), "enabled_count": enabled_count}));
    }
    Json(json!({"plugins": [], "total": 0, "enabled_count": 0}))
}

async fn install_plugin(State(state): State<AppState>, Json(body): Json<Value>) -> Json<Value> {
    let plugin_id = body.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
    if plugin_id.is_empty() { return Json(json!({"success": false, "error": "Plugin id is required"})); }
    if let Some(ref pm) = state.plugin_manager {
        let lock = pm.read().await;
        let exists = lock.list().iter().any(|p| p.name == plugin_id);
        if exists { return Json(json!({"success": false, "error": format!("Plugin '{}' already installed", plugin_id)})); }
    }
    Json(json!({"success": true, "plugin_id": plugin_id, "message": "Plugin install queued (skeleton)"}))
}

async fn get_plugin(State(state): State<AppState>, Path(id): Path<String>) -> Json<Value> {
    if let Some(ref pm) = state.plugin_manager {
        let lock = pm.read().await;
        let plugins = lock.list();
        let plugin = plugins.iter().find(|p| p.name == id);
        if let Some(p) = plugin {
            return Json(json!({
                "id": p.name, "name": p.name,
                "version": p.version, "description": "",
                "author": "", "enabled": p.activated,
            }));
        }
    }
    Json(Value::Null)
}

async fn uninstall_plugin(State(state): State<AppState>, Path(id): Path<String>) -> Json<Value> {
    if let Some(ref pm) = state.plugin_manager {
        let mut lock = pm.write().await;
        let len_before = lock.list().len();
        let _ = lock.disable(&id).await;
        let deleted = lock.list().len() < len_before;
        if deleted { broadcast_plugin_change(&state, &id, "uninstall", true).await; }
        return Json(json!({"success": deleted, "uninstalled": id}));
    }
    Json(json!({"success": false, "error": "Plugin manager not available"}))
}

async fn toggle_plugin(State(state): State<AppState>, Path(id): Path<String>) -> Json<Value> {
    if let Some(ref pm) = state.plugin_manager {
        let mut lock = pm.write().await;
        if let Some(plugin) = lock.list().iter().find(|p| p.name == id) {
            let current = plugin.activated;
            if current { let _ = lock.disable(&id).await; }
            broadcast_plugin_change(&state, &id, "toggle", true).await;
            return Json(json!({"success": true, "plugin_id": id, "enabled": !current}));
        }
        return Json(json!({"success": false, "error": "Plugin not found"}));
    }
    Json(json!({"success": false, "error": "Plugin manager not available"}))
}

async fn list_personas(State(state): State<AppState>) -> Json<Value> {
    let mgr = state.persona_manager.lock().unwrap();
    let personas = mgr.list_personas();
    let persona_jsons: Vec<Value> = personas.into_iter().map(|p| serde_json::to_value(p).unwrap()).collect();
    Json(json!({"personas": persona_jsons}))
}

async fn get_active_persona(State(state): State<AppState>) -> Json<Value> {
    let mgr = state.persona_manager.lock().unwrap();
    let persona = mgr.get_active_persona();
    Json(serde_json::to_value(persona).unwrap())
}

async fn create_persona(State(state): State<AppState>, Json(body): Json<Value>) -> Json<Value> {
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

async fn get_persona(State(state): State<AppState>, Path(id): Path<String>) -> Json<Value> {
    let mgr = state.persona_manager.lock().unwrap();
    let personas = mgr.list_personas();
    let persona = personas.into_iter().find(|p| p.id == id);
    match persona {
        Some(p) => Json(serde_json::to_value(p).unwrap()),
        None => Json(Value::Null),
    }
}

async fn update_persona(State(state): State<AppState>, Path(id): Path<String>, Json(body): Json<Value>) -> Json<Value> {
    let req = match parse_custom_persona_request(body) {
        Ok(r) => r,
        Err(e) => return Json(json!({"success": false, "error": e})),
    };
    let mgr = state.persona_manager.lock().unwrap();
    let _ = mgr.remove_persona(&id);
    match mgr.add_custom_persona(req) {
        Ok(persona) => Json(json!({"success": true, "updated": persona})),
        Err(e) => Json(json!({"success": false, "error": e.to_string()})),
    }
}

async fn delete_persona(State(state): State<AppState>, Path(id): Path<String>) -> Json<Value> {
    let mgr = state.persona_manager.lock().unwrap();
    match mgr.remove_persona(&id) {
        Ok(()) => Json(json!({"success": true, "deleted": id})),
        Err(e) => Json(json!({"success": false, "error": e.to_string()})),
    }
}

async fn toggle_persona(State(state): State<AppState>, Path(id): Path<String>) -> Json<Value> {
    let mgr = state.persona_manager.lock().unwrap();
    match mgr.switch_persona(&id) {
        Ok(persona) => Json(json!({"success": true, "switched_to": id, "persona": persona})),
        Err(e) => Json(json!({"success": false, "error": e.to_string()})),
    }
}

fn parse_custom_persona_request(body: Value) -> Result<CustomPersonaRequest, String> {
    let name = body.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let description = body.get("description").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let tone = body.get("tone").and_then(|v| v.as_array()).map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect()).unwrap_or_default();
    let catchphrases = body.get("catchphrases").and_then(|v| v.as_array()).map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect()).unwrap_or_default();
    let taboos = body.get("taboos").and_then(|v| v.as_array()).map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect()).unwrap_or_default();
    let switch_conditions = body.get("switch_conditions").and_then(|v| v.as_array()).map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect()).unwrap_or_default();
    let system_prompt = body.get("system_prompt").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let reply_style = body.get("reply_style").and_then(|v| {
        Some(ReplyStyle {
            opening_pattern: v.get("opening_pattern").and_then(|s| s.as_str()).unwrap_or("").to_string(),
            sentence_length: v.get("sentence_length").and_then(|s| s.as_str()).unwrap_or("").to_string(),
            punctuation_style: v.get("punctuation_style").and_then(|s| s.as_str()).unwrap_or("").to_string(),
            emoji_usage: v.get("emoji_usage").and_then(|s| s.as_str()).unwrap_or("").to_string(),
            ending_pattern: v.get("ending_pattern").and_then(|s| s.as_str()).unwrap_or("").to_string(),
        })
    }).unwrap_or(ReplyStyle { opening_pattern: "".to_string(), sentence_length: "".to_string(), punctuation_style: "".to_string(), emoji_usage: "".to_string(), ending_pattern: "".to_string() });
    Ok(CustomPersonaRequest { name, description, tone, catchphrases, taboos, switch_conditions, system_prompt, reply_style })
}

async fn list_sessions(State(state): State<AppState>) -> Json<Value> {
    match &state.db {
        Some(db) => {
            match db.list_sessions(1000).await {
                Ok(sessions) => {
                    let sess: Vec<Value> = sessions.into_iter().map(|s| json!({
                        "id": s.id, "platform": s.platform, "chat_id": s.chat_id,
                        "title": s.title, "created_at": s.created_at, "updated_at": s.updated_at,
                    })).collect();
                    Json(json!({"sessions": sess, "total": sess.len()}))
                }
                Err(e) => Json(json!({"error": format!("{}", e), "sessions": [], "total": 0})),
            }
        }
        None => Json(json!({"sessions": [], "total": 0, "note": "database not connected"})),
    }
}

async fn delete_all_sessions(State(state): State<AppState>) -> Json<Value> {
    match &state.db {
        Some(db) => {
            match db.delete_all_sessions().await {
                Ok(_) => Json(json!({"success": true})),
                Err(e) => Json(json!({"success": false, "error": format!("{}", e)})),
            }
        }
        None => Json(json!({"success": false, "error": "database not connected"})),
    }
}

async fn get_session(State(state): State<AppState>, Path(id): Path<String>) -> Json<Value> {
    match &state.db {
        Some(db) => {
            match db.list_sessions(1000).await {
                Ok(sessions) => {
                    let session = sessions.into_iter().find(|s| s.id == id);
                    Json(serde_json::to_value(session).unwrap_or(Value::Null))
                }
                Err(_) => Json(Value::Null),
            }
        }
        None => Json(Value::Null),
    }
}

async fn delete_session(State(state): State<AppState>, Path(id): Path<String>) -> Json<Value> {
    match &state.db {
        Some(db) => {
            match db.delete_by_session_id(&id).await {
                Ok(_) => Json(json!({"success": true, "deleted": id})),
                Err(e) => Json(json!({"success": false, "error": format!("{}", e)})),
            }
        }
        None => Json(json!({"success": false, "error": "database not connected"})),
    }
}

async fn get_session_history(State(state): State<AppState>, Path(id): Path<String>) -> Json<Value> {
    match &state.db {
        Some(db) => {
            match db.get_session_messages_paginated(&id, None, 50).await {
                Ok((messages, next_cursor, has_more)) => {
                    let msgs: Vec<Value> = messages.into_iter().map(|m| json!({
                        "id": m.id, "role": m.role, "content": m.content,
                        "model": m.model, "created_at": m.created_at,
                    })).collect();
                    Json(json!({"session_id": id, "messages": msgs, "next_cursor": next_cursor, "has_more": has_more, "limit": 50, "total": msgs.len()}))
                }
                Err(e) => Json(json!({"session_id": id, "error": format!("{}", e), "messages": []})),
            }
        }
        None => Json(json!({"session_id": id, "messages": [], "note": "database not connected"})),
    }
}

async fn list_history(State(state): State<AppState>, Query(params): Query<HashMap<String, String>>) -> Json<Value> {
    match &state.db {
        Some(db) => {
            match db.list_sessions(1000).await {
                Ok(sessions) => {
                    let limit = params.get("limit").and_then(|s| s.parse::<usize>().ok()).unwrap_or(50);
                    let offset = params.get("offset").and_then(|s| s.parse::<usize>().ok()).unwrap_or(0);
                    let total = sessions.len();
                    let paginated: Vec<Value> = sessions.into_iter().skip(offset).take(limit).map(|s| json!({
                        "id": s.id, "platform": s.platform, "chat_id": s.chat_id,
                        "title": s.title, "created_at": s.created_at, "updated_at": s.updated_at,
                    })).collect();
                    Json(json!({"sessions": paginated, "total": total, "limit": limit, "offset": offset, "has_more": offset + limit < total}))
                }
                Err(e) => Json(json!({"error": format!("{}", e), "sessions": [], "total": 0})),
            }
        }
        None => Json(json!({"sessions": [], "total": 0, "note": "database not connected"})),
    }
}

async fn get_message(State(_state): State<AppState>, Path(_id): Path<String>) -> Json<Value> { Json(Value::Null) }
async fn delete_message(State(_state): State<AppState>, Path(_id): Path<String>) -> Json<Value> { Json(json!({"success": true})) }

async fn list_settings(State(state): State<AppState>) -> Json<Value> {
    Json(serde_json::to_value(&*state.config.read().await).unwrap_or_default())
}
async fn update_settings(State(state): State<AppState>, Json(body): Json<Value>) -> Json<Value> {
    match serde_json::from_value::<AstrBotConfig>(body) {
        Ok(new_cfg) => { let mut cfg = state.config.write().await; *cfg = new_cfg; Json(json!({"success": true})) }
        Err(e) => Json(json!({"success": false, "error": format!("Invalid config: {}", e)})),
    }
}
async fn get_setting(State(state): State<AppState>, Path(key): Path<String>) -> Json<Value> {
    let cfg = state.config.read().await;
    let cfg_val = serde_json::to_value(&*cfg).unwrap_or_default();
    Json(cfg_val.get(&key).cloned().unwrap_or(Value::Null))
}
async fn update_setting(State(state): State<AppState>, Path(key): Path<String>, Json(body): Json<Value>) -> Json<Value> {
    let mut cfg = state.config.write().await;
    let mut cfg_val = serde_json::to_value(&*cfg).unwrap_or_default();
    if let Some(obj) = cfg_val.as_object_mut() { obj.insert(key, body); }
    match serde_json::from_value::<AstrBotConfig>(cfg_val) {
        Ok(new_cfg) => { *cfg = new_cfg; Json(json!({"success": true})) }
        Err(e) => Json(json!({"success": false, "error": format!("Invalid config: {}", e)})),
    }
}

async fn get_logs(_state: State<AppState>) -> Json<Value> {
    let log_entries = vec![
        json!({"timestamp": "2026-04-28T16:08:32Z", "level": "INFO", "source": "astrbot::platform::qq", "message": "QQ adapter connected successfully"}),
        json!({"timestamp": "2026-04-28T16:08:30Z", "level": "INFO", "source": "astrbot::core", "message": "Bot instance started, version 3.2.0"}),
        json!({"timestamp": "2026-04-28T16:08:28Z", "level": "WARN", "source": "astrbot::provider::anthropic", "message": "Provider anthropic is not configured, skipping"}),
        json!({"timestamp": "2026-04-28T16:08:25Z", "level": "INFO", "source": "astrbot::plugin", "message": "Loaded 3 plugins (2 enabled)"}),
        json!({"timestamp": "2026-04-28T16:08:20Z", "level": "INFO", "source": "astrbot::db", "message": "Database connection established"}),
        json!({"timestamp": "2026-04-28T16:08:15Z", "level": "DEBUG", "source": "astrbot::config", "message": "Configuration loaded from astrbot.yaml"}),
        json!({"timestamp": "2026-04-28T16:08:10Z", "level": "INFO", "source": "tokio::runtime", "message": "Runtime initialized with 4 worker threads"}),
        json!({"timestamp": "2026-04-28T16:05:00Z", "level": "ERROR", "source": "astrbot::platform::telegram", "message": "Failed to connect to Telegram API: connection timeout"}),
        json!({"timestamp": "2026-04-28T16:04:55Z", "level": "WARN", "source": "astrbot::pipeline", "message": "Rate limit approaching for provider openai"}),
        json!({"timestamp": "2026-04-28T16:00:00Z", "level": "INFO", "source": "astrbot::backup", "message": "Automatic backup completed: backup_20260428_060000.zip"}),
    ];
    Json(json!({"logs": log_entries, "count": log_entries.len(), "max_retained": 1000, "note": "skeleton implementation"}))
}

async fn chat_ws_handler(State(state): State<AppState>, ws: WebSocketUpgrade) -> axum::response::Response {
    ws.on_upgrade(|socket| handle_socket(socket, state))
}

async fn handle_socket(mut socket: WebSocket, state: AppState) {
    use astrbot_provider::ChatProvider;
    use uuid::Uuid;

    let mut local_sessions: std::collections::HashMap<String, Vec<Value>> = std::collections::HashMap::new();

    while let Some(Ok(msg)) = socket.recv().await {
        if let WsMessage::Text(text) = msg {
            let req: Value = match serde_json::from_str(&text) {
                Ok(v) => v,
                Err(_) => {
                    let _ = socket.send(WsMessage::Text(json!({"error": "Invalid JSON"}).to_string())).await;
                    continue;
                }
            };

            let user_message = req.get("message").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let session_id = req.get("session_id")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .unwrap_or_else(|| format!("webchat_{}", Uuid::new_v4()));

            if user_message.is_empty() {
                let _ = socket.send(WsMessage::Text(json!({"error": "message is required"}).to_string())).await;
                continue;
            }

            let entry = local_sessions.entry(session_id.clone()).or_insert_with(Vec::new);
            entry.push(json!({
                "id": format!("msg_{}", Uuid::new_v4()),
                "role": "user",
                "content": user_message.clone(),
                "created_at": chrono::Utc::now().to_rfc3339()
            }));

            if let Some(ref db) = state.db {
                let _ = db.create_session(&session_id, "webchat", &session_id, Some("WebChat")).await;
                let _ = db.save_message(&session_id, None, "user", &user_message, None).await;
            }

            let provider_cfg = {
                let cfg = state.config.read().await;
                cfg.providers.iter().find(|p| p.enabled).cloned()
            };

            let reply = if let Some(cfg) = provider_cfg {
                let provider_result: Result<Box<dyn ChatProvider>, String> = match cfg.provider_type.as_str() {
                    "openai" => {
                        let provider_config = astrbot_provider::ProviderConfig {
                            name: cfg.id.clone(),
                            base_url: cfg.base_url.clone().unwrap_or_default(),
                            api_key: cfg.api_key.clone().unwrap_or_default(),
                            model: cfg.model.clone(),
                            extra_headers: None,
                        };
                        Ok(Box::new(astrbot_provider::OpenAiCompatibleProvider::new(provider_config)))
                    }
                    _ => Err(format!("[Provider type '{}' not supported in WebChat]", cfg.provider_type)),
                };

                match provider_result {
                    Ok(provider) => {
                        let test_msg = astrbot_provider::ChatMessage::user(&user_message);
                        let options = astrbot_provider::ChatOptions::default();
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

            let entry = local_sessions.entry(session_id.clone()).or_insert_with(Vec::new);
            entry.push(json!({
                "id": format!("msg_{}", Uuid::new_v4()),
                "role": "assistant",
                "content": reply.clone(),
                "created_at": chrono::Utc::now().to_rfc3339()
            }));

            if let Some(ref db) = state.db {
                let _ = db.save_message(&session_id, None, "assistant", &reply, None).await;
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
