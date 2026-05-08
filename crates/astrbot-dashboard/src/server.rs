use axum::{
    Router,
    routing::{get, post, delete},
    extract::{Path, State, Query, WebSocketUpgrade},
    extract::ws::{WebSocket, Message as WsMessage},
    Json,
};
use crate::api::{get_enhanced_status, update_config_with_broadcast, broadcast_config_update, broadcast_provider_status, broadcast_plugin_change};
use crate::app_state::AppState;
use crate::kb_api::{list_kb_collections, search_kb, index_kb, delete_kb_doc};
use astrbot_core::config::AstrBotConfig;
use astrbot_core::provider::{ChatMessage as CoreChatMessage, ChatConfig};
use astrbot_persona::{PersonaManager, CustomPersonaRequest, ReplyStyle};
use astrbot_provider::{ChatProvider, ChatMessage as ProviderChatMessage, ChatOptions, ProviderConfig};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::net::SocketAddr;
use tokio::sync::RwLock;

pub async fn start_server(state: AppState) {
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
        .route("/api/sessions/:id", get(get_session).delete(delete_session_stub))
        .route("/api/sessions/:id/history", get(get_session_history))
        .route("/api/history", get(list_history_stub))
        .route("/api/history/:id", get(get_message_stub).delete(delete_message_stub))
        .route("/api/settings", get(list_settings).put(update_settings))
        .route("/api/settings/:key", get(get_setting).put(update_setting))
        .route("/api/logs", get(get_logs))
        .route("/api/events", get(crate::sse::events_handler))
        .route("/api/knowledge-base", get(list_kb_collections))
        .route("/api/knowledge-base/search", post(search_kb))
        .route("/api/knowledge-base/index", post(index_kb))
        .route("/api/knowledge-base/:id", delete(delete_kb_doc))
        .route("/ws/chat", get(chat_ws_handler))
        .fallback_service(
            tower_http::services::ServeDir::new("./dashboard/dist")
                .fallback(tower_http::services::ServeFile::new("./dashboard/dist/index.html"))
        )
        .with_state(state)
}

async fn get_detailed_status(State(state): State<AppState>) -> Json<Value> {
    let config = serde_json::to_value(&*state.config.read().await).unwrap_or_default();
    let providers_count = {
        let lock = state.provider_manager.read().await;
        lock.list().len()
    };
    let platforms_count = {
        let cfg = state.config.read().await;
        cfg.platforms.len()
    };
    let plugins_count = {
        let lock = state.plugin_manager.read().await;
        lock.list().len()
    };
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
        "providers_count": providers_count,
        "platforms_count": platforms_count,
        "plugins_count": plugins_count,
        "personas_count": personas.len(),
        "active_persona": active_persona.id,
        "active_persona_name": active_persona.name,
    }))
}

async fn get_config(State(state): State<AppState>) -> Json<Value> {
    Json(serde_json::to_value(&*state.config.read().await).unwrap_or_default())
}

async fn get_config_key(State(state): State<AppState>, Path(key): Path<String>) -> Json<Value> {
    let cfg = state.config.read().await;
    let cfg_val = serde_json::to_value(&*cfg).unwrap_or_default();
    Json(cfg_val.get(&key).cloned().unwrap_or(Value::Null))
}

async fn update_config_key(State(state): State<AppState>, Path(key): Path<String>, Json(body): Json<Value>) -> Json<Value> {
    let mut cfg = state.config.write().await;
    let mut cfg_val = serde_json::to_value(&*cfg).unwrap_or_default();
    let key_clone = key.clone();
    if let Some(obj) = cfg_val.as_object_mut() {
        obj.insert(key, body);
    }
    match serde_json::from_value::<AstrBotConfig>(cfg_val) {
        Ok(new_cfg) => {
            *cfg = new_cfg;
            broadcast_config_update(&state, vec![key_clone]).await;
            Json(json!({"success": true}))
        }
        Err(e) => Json(json!({"success": false, "error": format!("Invalid config: {}", e)})),
    }
}

async fn list_providers(State(state): State<AppState>) -> Json<Value> {
    let lock = state.provider_manager.read().await;
    let providers = lock.list();
    let items: Vec<Value> = providers.iter().map(|p| {
        json!({
            "id": p.id(),
            "name": p.name(),
        })
    }).collect();
    Json(json!({"providers": items}))
}

async fn get_provider(State(state): State<AppState>, Path(id): Path<String>) -> Json<Value> {
    let lock = state.provider_manager.read().await;
    let providers = lock.list();
    let provider = providers.iter().find(|p| p.id() == &id || p.name() == &id);
    Json(provider.map(|p| json!({"id": p.id(), "name": p.name()})).unwrap_or(Value::Null))
}

async fn update_provider(State(state): State<AppState>, Path(id): Path<String>, Json(body): Json<Value>) -> Json<Value> {
    let mut cfg = state.config.write().await;
    let providers_val = serde_json::to_value(&cfg.providers).unwrap_or(json!([]));
    let mut providers = providers_val.as_array().cloned().unwrap_or_default();
    match providers.iter().position(|p| {
        p.get("id").and_then(|v| v.as_str()) == Some(&id)
            || p.get("name").and_then(|v| v.as_str()) == Some(&id)
    }) {
        Some(i) => {
            let mut updated = providers[i].clone();
            if let Some(obj) = updated.as_object_mut() {
                if let Some(body_obj) = body.as_object() {
                    for (k, v) in body_obj { obj.insert(k.clone(), v.clone()); }
                }
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
    providers.retain(|p| {
        p.get("id").and_then(|v| v.as_str()) != Some(&id)
            && p.get("name").and_then(|v| v.as_str()) != Some(&id)
    });
    let deleted = providers.len() < len_before;
    cfg.providers = providers.iter().filter_map(|v| serde_json::from_value(v.clone()).ok()).collect();
    Json(json!({"success": deleted, "deleted": id}))
}

async fn test_provider(State(state): State<AppState>, Path(id): Path<String>) -> Json<Value> {
    let cfg = {
        let lock = state.provider_manager.read().await;
        let providers = lock.list();
        let provider_cfg = providers.iter().find(|p| p.id() == &id || p.name() == &id);
        match provider_cfg {
            Some(p) => {
                let mut map = serde_json::Map::new();
                map.insert("id".to_string(), json!(p.id()));
                map.insert("name".to_string(), json!(p.name()));
                Value::Object(map)
            }
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
    let test_msg = ProviderChatMessage::user("Hello, this is a connectivity test from AstrBot Dashboard.");
    let options = ChatOptions::default();
    let result = _test_provider_chat(provider_type, config, id, test_msg, options).await;
    let latency_ms = start.elapsed().as_millis() as u64;

    let mut response = result;
    if let Some(obj) = response.as_object_mut() {
        obj.insert("latency_ms".to_string(), json!(latency_ms));
    }
    Json(response)
}

async fn _test_provider_chat(provider_type: &str, config: ProviderConfig, id: String, test_msg: ProviderChatMessage, options: ChatOptions) -> Value {
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
    let platform = platforms_val.as_array().and_then(|arr| arr.iter().find(|p| p.get("id").and_then(|v| v.as_str()) == Some(&id)));
    Json(platform.cloned().unwrap_or(Value::Null))
}

async fn update_platform(State(state): State<AppState>, Path(id): Path<String>, Json(body): Json<Value>) -> Json<Value> {
    let mut cfg = state.config.write().await;
    let platforms_val = serde_json::to_value(&cfg.platforms).unwrap_or(json!([]));
    let mut platforms = platforms_val.as_array().cloned().unwrap_or_default();
    if let Some(idx) = platforms.iter().position(|p| p.get("id").and_then(|v| v.as_str()) == Some(&id)) {
        let mut updated = platforms[idx].clone();
        if let Some(obj) = updated.as_object_mut() {
            if let Some(body_obj) = body.as_object() {
                for (key, val) in body_obj.iter() { obj.insert(key.clone(), val.clone()); }
            }
            obj.insert("updated_at".to_string(), json!(format!("{:?}", std::time::SystemTime::now())));
        }
        platforms[idx] = updated.clone();
        cfg.platforms = platforms.iter().filter_map(|v| serde_json::from_value(v.clone()).ok()).collect();
        return Json(json!({"success": true, "updated": id, "platform": updated}));
    }
    Json(json!({"success": false, "error": format!("Platform '{}' not found", id)}))
}
async fn list_plugins(State(state): State<AppState>) -> Json<Value> {
    let lock = state.plugin_manager.read().await;
    let items = lock.list();
    let count = items.len();
    let items_json: Vec<Value> = items.iter().map(|s| {
        json!({
            "name": s.name,
            "version": s.version,
            "lifecycle": format!("{:?}", s.lifecycle),
            "loaded_at": s.loaded_at,
            "activated": s.activated,
        })
    }).collect();
    Json(json!({"plugins": items_json, "count": count}))
}

async fn get_plugin(State(state): State<AppState>, Path(id): Path<String>) -> Json<Value> {
    let lock = state.plugin_manager.read().await;
    let items = lock.list();
    let plugin = items.iter().find(|s| s.name == id);
    Json(plugin.map(|s| json!({
        "name": s.name,
        "version": s.version,
        "lifecycle": format!("{:?}", s.lifecycle),
        "loaded_at": s.loaded_at,
        "activated": s.activated,
    })).unwrap_or(Value::Null))
}

async fn install_plugin(State(state): State<AppState>, Json(body): Json<Value>) -> Json<Value> {
    let identifier = body.get("identifier").and_then(|v| v.as_str()).unwrap_or("").to_string();
    if identifier.is_empty() {
        return Json(json!({"success": false, "error": "Missing identifier"}));
    }
    // TODO: implement real install via plugin_manager
    Json(json!({"success": true, "installed": identifier, "note": "stub"}))
}

async fn uninstall_plugin(State(state): State<AppState>, Path(id): Path<String>) -> Json<Value> {
    // TODO: implement real uninstall
    Json(json!({"success": true, "uninstalled": id, "note": "stub"}))
}

async fn toggle_plugin(State(state): State<AppState>, Path(id): Path<String>, Json(body): Json<Value>) -> Json<Value> {
    let _enable = body.get("enable").and_then(|v| v.as_bool()).unwrap_or(true);
    // TODO: implement real toggle
    Json(json!({"success": true, "id": id, "enabled": _enable, "note": "stub"}))
}

async fn list_personas(State(state): State<AppState>) -> Json<Value> {
    let mgr = state.persona_manager.lock().unwrap();
    let personas = mgr.list_personas();
    let items: Vec<Value> = personas.iter().map(|p| {
        json!({
            "id": p.id,
            "name": p.name,
            "description": p.description,
        })
    }).collect();
    Json(json!({"personas": items, "count": items.len()}))
}

async fn get_active_persona(State(state): State<AppState>) -> Json<Value> {
    let mgr = state.persona_manager.lock().unwrap();
    let p = mgr.get_active_persona();
    Json(json!({
        "id": p.id,
        "name": p.name,
        "description": p.description,
    }))
}

async fn get_persona(State(state): State<AppState>, Path(id): Path<String>) -> Json<Value> {
    let mgr = state.persona_manager.lock().unwrap();
    let personas = mgr.list_personas();
    let persona = personas.iter().find(|p| p.id == id);
    Json(persona.map(|p| json!({
        "id": p.id,
        "name": p.name,
        "description": p.description,
    })).unwrap_or(Value::Null))
}

async fn create_persona(State(state): State<AppState>, Json(body): Json<CustomPersonaRequest>) -> Json<Value> {
    let mgr = state.persona_manager.lock().unwrap();
    match mgr.add_custom_persona(body) {
        Ok(p) => Json(json!({"success": true, "id": p.id})),
        Err(e) => Json(json!({"success": false, "error": e.to_string()})),
    }
}

async fn update_persona(State(state): State<AppState>, Path(id): Path<String>, Json(body): Json<CustomPersonaRequest>) -> Json<Value> {
    let mut mgr = state.persona_manager.lock().unwrap();
    // Remove old, add new with same id
    let _ = mgr.remove_persona(&id);
    drop(mgr);
    let mgr = state.persona_manager.lock().unwrap();
    match mgr.add_custom_persona(body) {
        Ok(p) => Json(json!({"success": true, "updated": p.id})),
        Err(e) => Json(json!({"success": false, "error": e.to_string()})),
    }
}

async fn delete_persona(State(state): State<AppState>, Path(id): Path<String>) -> Json<Value> {
    let mgr = state.persona_manager.lock().unwrap();
    match mgr.remove_persona(&id) {
        Ok(_) => Json(json!({"success": true, "deleted": id})),
        Err(e) => Json(json!({"success": false, "error": e.to_string()})),
    }
}

async fn toggle_persona(State(state): State<AppState>, Path(id): Path<String>) -> Json<Value> {
    let mgr = state.persona_manager.lock().unwrap();
    match mgr.switch_persona(&id) {
        Ok(_) => Json(json!({"success": true, "active": id})),
        Err(e) => Json(json!({"success": false, "error": e.to_string()})),
    }
}

async fn list_sessions(State(state): State<AppState>) -> Json<Value> {
    match &state.db {
        Some(db) => {
            match db.list_sessions(100).await {
                Ok(sessions) => {
                    let items: Vec<Value> = sessions.iter().map(|s| {
                        json!({
                            "id": s.id,
                            "platform": s.platform,
                            "chat_id": s.chat_id,
                            "title": s.title,
                        })
                    }).collect();
                    Json(json!({"sessions": items, "count": items.len()}))
                }
                Err(e) => Json(json!({"success": false, "error": e.to_string()})),
            }
        }
        None => Json(json!({"sessions": [], "count": 0})),
    }
}

async fn get_session(State(state): State<AppState>, Path(id): Path<String>) -> Json<Value> {
    match &state.db {
        Some(db) => {
            match db.get_session(&id).await {
                Ok(Some(session)) => Json(json!({
                    "id": session.id,
                    "platform": session.platform,
                    "chat_id": session.chat_id,
                    "title": session.title,
                })),
                Ok(None) => Json(json!({"success": false, "error": "Session not found"})),
                Err(e) => Json(json!({"success": false, "error": e.to_string()})),
            }
        }
        None => Json(json!({"success": false, "error": "Database not configured"})),
    }
}

async fn delete_session_stub(State(state): State<AppState>, Path(id): Path<String>) -> Json<Value> {
    match &state.db {
        Some(db) => match db.delete_by_session_id(&id).await {
            Ok(count) => Json(json!({"success": true, "deleted": id, "messages_removed": count})),
            Err(e) => Json(json!({"success": false, "error": e.to_string()})),
        },
        None => Json(json!({"success": false, "error": "Database not configured"})),
    }
}

async fn delete_all_sessions(State(state): State<AppState>) -> Json<Value> {
    match &state.db {
        Some(db) => match db.delete_all_sessions().await {
            Ok(count) => Json(json!({"success": true, "deleted_count": count})),
            Err(e) => Json(json!({"success": false, "error": e.to_string()})),
        },
        None => Json(json!({"success": false, "error": "Database not configured"})),
    }
}

async fn get_session_history(State(state): State<AppState>, Path(id): Path<String>, Query(params): Query<HashMap<String, String>>) -> Json<Value> {
    let limit = params.get("limit").and_then(|v| v.parse::<usize>().ok()).unwrap_or(50);
    match &state.db {
        Some(db) => {
            match db.get_session_messages(&id, limit as i64).await {
                Ok(messages) => {
                    let items: Vec<Value> = messages.iter().map(|m| {
                        json!({
                            "id": m.id,
                            "role": m.role,
                            "content": m.content,
                            "created_at": m.created_at,
                        })
                    }).collect();
                    Json(json!({"session_id": id, "messages": items, "count": items.len()}))
                }
                Err(e) => Json(json!({"success": false, "error": e.to_string()})),
            }
        }
        None => Json(json!({"success": false, "error": "Database not configured"})),
    }
}

async fn list_history_stub(State(state): State<AppState>, Query(_params): Query<HashMap<String, String>>) -> Json<Value> {
    Json(json!({"messages": [], "count": 0, "note": "stub"}))
}

async fn get_message_stub(State(state): State<AppState>, Path(_id): Path<String>) -> Json<Value> {
    Json(json!({"success": false, "error": "Not implemented", "note": "stub"}))
}

async fn delete_message_stub(State(state): State<AppState>, Path(_id): Path<String>) -> Json<Value> {
    Json(json!({"success": false, "error": "Not implemented", "note": "stub"}))
}

async fn list_settings(State(state): State<AppState>) -> Json<Value> {
    let cfg = state.config.read().await;
    Json(json!({
        "nickname": cfg.nickname,
        "prefixes": cfg.prefixes,
        "log_level": cfg.log_level,
        "database_url": cfg.database_url,
    }))
}

async fn update_settings(State(state): State<AppState>, Json(body): Json<Value>) -> Json<Value> {
    let mut cfg = state.config.write().await;
    if let Some(nickname) = body.get("nickname").and_then(|v| v.as_str()) {
        cfg.nickname = nickname.to_string();
    }
    if let Some(prefixes) = body.get("prefixes").and_then(|v| v.as_array()) {
        cfg.prefixes = prefixes.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect();
    }
    if let Some(log_level) = body.get("log_level").and_then(|v| v.as_str()) {
        cfg.log_level = log_level.to_string();
    }
    Json(json!({"success": true}))
}

async fn get_setting(State(state): State<AppState>, Path(key): Path<String>) -> Json<Value> {
    let cfg = state.config.read().await;
    let val = match key.as_str() {
        "nickname" => json!(cfg.nickname),
        "prefixes" => json!(cfg.prefixes),
        "log_level" => json!(cfg.log_level),
        "database_url" => json!(cfg.database_url),
        _ => Value::Null,
    };
    Json(val)
}

async fn update_setting(State(state): State<AppState>, Path(key): Path<String>, Json(body): Json<Value>) -> Json<Value> {
    let mut cfg = state.config.write().await;
    match key.as_str() {
        "nickname" => {
            if let Some(v) = body.as_str() { cfg.nickname = v.to_string(); }
        }
        "prefixes" => {
            if let Some(arr) = body.as_array() {
                cfg.prefixes = arr.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect();
            }
        }
        "log_level" => {
            if let Some(v) = body.as_str() { cfg.log_level = v.to_string(); }
        }
        "database_url" => {
            if let Some(v) = body.as_str() { cfg.database_url = v.to_string(); }
        }
        _ => {}
    }
    Json(json!({"success": true}))
}

async fn get_logs(State(state): State<AppState>) -> Json<Value> {
    let logs = match &state.log_buffer {
        Some(buf) => buf.get_recent(500),
        None => vec![],
    };
    Json(json!({"logs": logs}))
}

async fn chat_ws_handler(ws: WebSocketUpgrade, State(state): State<AppState>) -> impl axum::response::IntoResponse {
    ws.on_upgrade(move |socket| handle_chat_socket(socket, state))
}

#[derive(Debug, serde::Deserialize)]
struct ChatRequest {
    content: String,
    #[serde(default)]
    provider_id: Option<String>,
    #[serde(default)]
    stream: bool,
    #[serde(default)]
    session_id: Option<String>,
}

#[derive(Debug, serde::Serialize)]
struct ChatChunk {
    #[serde(rename = "type")]
    kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    delta: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    finish_reason: Option<String>,
}

async fn handle_chat_socket(mut socket: WebSocket, state: AppState) {
    loop {
        match socket.recv().await {
            Some(Ok(WsMessage::Text(text))) => {
                let req: ChatRequest = match serde_json::from_str(&text) {
                    Ok(r) => r,
                    Err(e) => {
                        let err = json!({"type": "error", "message": format!("Invalid request: {}", e)});
                        let _ = socket.send(WsMessage::Text(err.to_string())).await;
                        continue;
                    }
                };

                // Hold provider manager lock during chat
                let lock = state.provider_manager.read().await;
                let providers = lock.list();
                let provider = if let Some(ref id) = req.provider_id {
                    providers.iter().find(|p| p.id() == id).copied()
                } else {
                    providers.first().copied()
                };

                let Some(provider) = provider else {
                    let err = json!({"type": "error", "message": "No provider available"});
                    let _ = socket.send(WsMessage::Text(err.to_string())).await;
                    continue;
                };

                let messages = vec![CoreChatMessage::user(&req.content)];
                let config = ChatConfig::default();

                if req.stream {
                    // Streaming mode — P0 fallback to non-streaming for now
                    // (provider.chat_stream returns !Unpin stream, needs proper pinning)
                    match provider.chat(messages, config).await {
                        Ok(response) => {
                            let msg = json!({"type": "chunk", "delta": response.content});
                            let _ = socket.send(WsMessage::Text(msg.to_string())).await;
                            let done = json!({"type": "done"});
                            if let Err(e) = socket.send(WsMessage::Text(done.to_string())).await {
                                tracing::warn!("WS send error: {}", e);
                                break;
                            }
                        }
                        Err(e) => {
                            let err = json!({"type": "error", "message": format!("Chat error: {}", e)});
                            let _ = socket.send(WsMessage::Text(err.to_string())).await;
                        }
                    }
                } else {
                    // Non-streaming mode
                    match provider.chat(messages, config).await {
                        Ok(response) => {
                            let msg = json!({"type": "response", "content": response.content});
                            if let Err(e) = socket.send(WsMessage::Text(msg.to_string())).await {
                                tracing::warn!("WS send error: {}", e);
                                break;
                            }
                        }
                        Err(e) => {
                            let err = json!({"type": "error", "message": format!("Chat error: {}", e)});
                            let _ = socket.send(WsMessage::Text(err.to_string())).await;
                        }
                    }
                }
            }
            Some(Ok(WsMessage::Close(_))) | None => {
                break;
            }
            _ => {}
        }
    }
}
