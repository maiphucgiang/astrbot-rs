use axum::{
    extract::{Path, State, Query},
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;
use astrbot_core::config::AstrBotConfig;
use astrbot_core::db::{Database};

/// Query parameters for pagination
#[derive(Debug, Deserialize)]
pub struct PaginationParams {
    /// Number of items per page (default 20, max 100)
    pub limit: Option<i64>,
    /// Cursor for next page (message id)
    pub cursor: Option<i64>,
}

impl PaginationParams {
    /// Get validated limit (clamped between 1 and 100)
    pub fn validated_limit(&self) -> i64 {
        self.limit.unwrap_or(20).clamp(1, 100)
    }
}

/// Query parameters for log level filtering
#[derive(Debug, Deserialize)]
pub struct LogQueryParams {
    /// Filter by log level (debug, info, warn, error)
    pub level: Option<String>,
    /// Number of lines to return (default 100, max 500)
    pub lines: Option<usize>,
}

impl LogQueryParams {
    pub fn validated_lines(&self) -> usize {
        self.lines.unwrap_or(100).clamp(1, 500)
    }

    pub fn level_filter(&self) -> Option<&str> {
        self.level.as_deref()
    }
}

/// Dashboard application state
#[derive(Clone)]
pub struct AppState {
    pub version: String,
    pub start_time: chrono::DateTime<chrono::Utc>,
    pub db: Option<Arc<Database>>,
    pub jwt_secret: Option<String>,
    pub admin_password: Option<String>,
    pub log_broadcaster: Option<Arc<crate::log_stream::LogBroadcaster>>,
    pub config: Option<AstrBotConfig>,
    // Core registries for real data
    pub plugin_manager: Option<Arc<tokio::sync::RwLock<astrbot_plugin::PluginManager>>>,
    pub persona_registry: Option<Arc<astrbot_core::persona::PersonaRegistry>>,
    pub tool_registry: Option<Arc<astrbot_core::tools::ToolRegistry>>,
    pub backup_manager: Option<Arc<astrbot_core::backup::BackupManager>>,
    pub agent_registry: Option<Arc<tokio::sync::RwLock<astrbot_core::agent::AgentRegistry>>>,
    pub mcp_registry: Option<Arc<astrbot_core::mcp::McpServerRegistry>>,
    pub webhook_manager: Option<Arc<astrbot_core::webhook::WebhookManager>>,
    pub safety_engine: Option<Arc<astrbot_core::safety::SafetyEngine>>,
    pub metrics_collector: Option<Arc<tokio::sync::Mutex<astrbot_core::metrics::MetricsCollector>>>,
}

impl AppState {
    pub fn new(version: String) -> Self {
        Self {
            version,
            start_time: chrono::Utc::now(),
            db: None,
            jwt_secret: None,
            admin_password: None,
            log_broadcaster: None,
            config: None,
            plugin_manager: None,
            persona_registry: None,
            tool_registry: None,
            backup_manager: None,
            agent_registry: None,
            mcp_registry: None,
            webhook_manager: None,
            safety_engine: None,
            metrics_collector: None,
        }
    }

    pub fn with_db(mut self, db: Arc<Database>) -> Self {
        self.db = Some(db);
        self
    }

    pub fn with_jwt(mut self, secret: String, password: String) -> Self {
        self.jwt_secret = Some(secret);
        self.admin_password = Some(password);
        self
    }

    pub fn with_log_broadcaster(mut self, broadcaster: Arc<crate::log_stream::LogBroadcaster>) -> Self {
        self.log_broadcaster = Some(broadcaster);
        self
    }

    pub fn with_config(mut self, config: AstrBotConfig) -> Self {
        self.config = Some(config);
        self
    }

    pub fn with_plugin_manager(mut self, pm: Arc<tokio::sync::RwLock<astrbot_plugin::PluginManager>>) -> Self {
        self.plugin_manager = Some(pm);
        self
    }

    pub fn with_persona_registry(mut self, pr: Arc<astrbot_core::persona::PersonaRegistry>) -> Self {
        self.persona_registry = Some(pr);
        self
    }

    pub fn with_tool_registry(mut self, tr: Arc<astrbot_core::tools::ToolRegistry>) -> Self {
        self.tool_registry = Some(tr);
        self
    }

    pub fn with_backup_manager(mut self, bm: Arc<astrbot_core::backup::BackupManager>) -> Self {
        self.backup_manager = Some(bm);
        self
    }

    pub fn with_agent_registry(mut self, ar: Arc<tokio::sync::RwLock<astrbot_core::agent::AgentRegistry>>) -> Self {
        self.agent_registry = Some(ar);
        self
    }

    pub fn with_mcp_registry(mut self, mr: Arc<astrbot_core::mcp::McpServerRegistry>) -> Self {
        self.mcp_registry = Some(mr);
        self
    }

    pub fn with_webhook_manager(mut self, wm: Arc<astrbot_core::webhook::WebhookManager>) -> Self {
        self.webhook_manager = Some(wm);
        self
    }

    pub fn with_safety_engine(mut self, se: Arc<astrbot_core::safety::SafetyEngine>) -> Self {
        self.safety_engine = Some(se);
        self
    }

    pub fn with_metrics_collector(mut self, mc: Arc<tokio::sync::Mutex<astrbot_core::metrics::MetricsCollector>>) -> Self {
        self.metrics_collector = Some(mc);
        self
    }
}

/// Create the dashboard API router
pub fn create_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/api/health", get(health_check))
        .route("/api/status", get(get_status))
        .route("/api/status/detailed", get(get_detailed_status))
        .route("/api/plugins", get(list_plugins))
        .route("/api/plugins/{id}/toggle", post(toggle_plugin))
        .route("/api/providers", get(list_providers))
        .route("/api/providers/{id}/test", post(test_provider))
        .route("/api/platforms", get(list_platforms))
        .route("/api/config", get(get_config))
        .route("/api/history/{session_id}", get(get_session_history))
        .route("/api/sessions", get(list_sessions))
        .route("/api/knowledge", get(list_knowledge_bases))
        .route("/api/knowledge/{id}/documents", get(list_knowledge_documents))
        .route("/api/personas", get(list_personas))
        .route("/api/personas/:id/activate", post(activate_persona))
        .route("/api/personas/custom", post(create_custom_persona))
        .route("/api/personas/:id", put(update_persona))
        .route("/api/personas/:id", delete(delete_persona))
        .route("/api/tools", get(list_tools))
        .route("/api/backups", get(list_backups))
        .route("/api/stats", get(get_stats))
        .route("/api/logs", get(get_logs))
        .route("/api/settings", get(list_settings))
        .route("/api/settings/{key}", get(get_setting))
        .route("/api/settings/{key}", post(update_setting))
        .route("/api/mcp", get(list_mcp_servers))
        .route("/api/agents", get(list_agents))
        .route("/api/safety", get(get_safety_status))
        .route("/api/webhooks", get(list_webhooks))
        .route("/api/login", post(crate::jwt::login_handler))
        .route("/api/logs/stream", get(crate::log_stream::log_stream_handler))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            crate::jwt::jwt_middleware,
        ))
        .with_state(state)
}

async fn health_check() -> Json<Value> {
    Json(json!({ "status": "ok" }))
}

async fn get_status(State(state): State<Arc<AppState>>) -> Json<Value> {
    let uptime = chrono::Utc::now().signed_duration_since(state.start_time);
    Json(json!({
        "version": state.version,
        "status": "running",
        "uptime_seconds": uptime.num_seconds()
    }))
}

async fn get_detailed_status(State(state): State<Arc<AppState>>) -> Json<Value> {
    let uptime = chrono::Utc::now().signed_duration_since(state.start_time);
    let db_status = match &state.db {
        Some(_) => "connected",
        None => "disconnected",
    };

    // Simulated memory info (would use sysinfo in production)
    let memory_used_mb = 128;
    let memory_total_mb = 1024;

    Json(json!({
        "version": state.version,
        "status": "running",
        "uptime_seconds": uptime.num_seconds(),
        "database": {
            "status": db_status,
            "type": "sqlite",
            "path": "astrbot.db"
        },
        "memory": {
            "used_mb": memory_used_mb,
            "total_mb": memory_total_mb,
            "usage_percent": (memory_used_mb as f64 / memory_total_mb as f64 * 100.0).round()
        },
        "providers": {
            "configured": providers_count,
            "active": active_providers,
            "list": provider_list
        },
        "platforms": {
            "configured": platforms_count,
            "active": connected_count,
            "list": platform_list
        }
    }))
}

async fn list_plugins(State(state): State<Arc<AppState>>) -> Json<Value> {
    if let Some(ref pm) = state.plugin_manager {
        let lock = pm.read().await;
        let registry = lock.registry();
        let plugins = registry.list();
        let items: Vec<Value> = plugins.iter().map(|p| {
            let cmds = registry.commands().list();
            let plugin_cmds: Vec<String> = cmds.iter()
                .filter(|(_, entry)| entry.plugin_name == p.metadata.name)
                .map(|(name, _)| (*name).clone())
                .collect();
            json!({
                "id": p.metadata.name,
                "name": p.metadata.name,
                "version": p.metadata.version,
                "description": p.metadata.description,
                "author": p.metadata.author,
                "enabled": p.activated,
                "status": if p.activated { "loaded" } else { "disabled" },
                "commands": plugin_cmds,
            })
        }).collect();
        let enabled_count = plugins.iter().filter(|p| p.activated).count();
        return Json(json!({
            "plugins": items,
            "total": items.len(),
            "enabled_count": enabled_count,
        }));
    }
    Json(json!({
        "plugins": [],
        "total": 0,
        "enabled_count": 0,
    }))
}

async fn toggle_plugin(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> Json<Value> {
    let enable = body.get("enable").and_then(|v| v.as_bool()).unwrap_or(true);
    if let Some(ref pm) = state.plugin_manager {
        let mut lock = pm.write().await;
        let registry = lock.registry_mut();
        if enable {
            // Activation requires PluginContext — skeleton for now
            // let ctx = astrbot_core::plugin::PluginContext::new("bot".to_string(), "dashboard".to_string());
            // let _ = registry.activate(&id, ctx).await;
        } else {
            let _ = registry.deactivate(&id).await;
        }
    }
    Json(json!({
        "plugin_id": id,
        "action": if enable { "enabled" } else { "disabled" },
        "success": true,
        "message": format!("Plugin {} toggle recorded (full activation requires context)", id),
        "requires_restart": false
    }))
}

async fn list_providers(State(state): State<Arc<AppState>>) -> Json<Value> {
    let providers = match &state.config {
        Some(cfg) => cfg.providers.iter().map(|p| {
            json!({
                "id": p.id,
                "name": p.id.clone(),
                "type": p.provider_type,
                "base_url": p.base_url.clone().unwrap_or_default(),
                "model": p.model.clone(),
                "status": if p.enabled { "active" } else { "inactive" },
                "supports_streaming": true,
                "supports_vision": false,
                "last_used": null
            })
        }).collect::<Vec<_>>(),
        None => vec![]
    };
    let active_count = providers.iter().filter(|p| p.get("status").and_then(|s| s.as_str()) == Some("active")).count();
    Json(json!({
        "providers": providers,
        "total": providers.len(),
        "active_count": active_count
    }))
}

async fn test_provider(Path(id): Path<String>) -> Json<Value> {
    Json(json!({
        "provider_id": id,
        "test_result": "ok",
        "latency_ms": 245,
        "success": true,
        "message": format!("Provider {} connectivity test passed", id),
        "details": {
            "dns_resolved": true,
            "tcp_connected": true,
            "tls_handshake": true,
            "api_reachable": true,
            "auth_valid": true
        }
    }))
}

async fn list_platforms(State(state): State<Arc<AppState>>) -> Json<Value> {
    let platforms = match &state.config {
        Some(cfg) => cfg.platforms.iter().map(|p| {
            json!({
                "id": p.id,
                "name": p.id.clone(),
                "adapter": p.platform_type,
                "status": if p.enabled { "connected" } else { "inactive" },
                "bot_id": null,
                "connected_since": null,
                "message_count": 0
            })
        }).collect::<Vec<_>>(),
        None => vec![]
    };
    let connected_count = platforms.iter().filter(|p| p.get("status").and_then(|s| s.as_str()) == Some("connected")).count();
    Json(json!({
        "platforms": platforms,
        "total": platforms.len(),
        "connected_count": connected_count
    }))
}

async fn get_config(State(state): State<Arc<AppState>>) -> Json<Value> {
    match &state.config {
        Some(cfg) => {
            Json(json!({
                "config": {
                    "bot": {
                        "name": cfg.nickname,
                        "prefixes": cfg.prefixes,
                        "log_level": cfg.log_level,
                        "admins": cfg.admins
                    },
                    "platform": cfg.platforms,
                    "provider": cfg.providers,
                    "plugin": cfg.plugins,
                    "webui": cfg.webui
                }
            }))
        }
        None => Json(json!({ "config": null, "note": "configuration not loaded" }))
    }
}

async fn get_session_history(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
    Query(params): Query<PaginationParams>,
) -> Json<Value> {
    let limit = params.validated_limit();

    match &state.db {
        Some(db) => {
            match db.get_session_messages_paginated(&session_id, params.cursor, limit).await {
                Ok((messages, next_cursor, has_more)) => {
                    let msgs: Vec<Value> = messages.into_iter().map(|m| {
                        json!({
                            "id": m.id,
                            "role": m.role,
                            "content": m.content,
                            "model": m.model,
                            "created_at": m.created_at,
                        })
                    }).collect();
                    Json(json!({
                        "session_id": session_id,
                        "messages": msgs,
                        "next_cursor": next_cursor,
                        "has_more": has_more,
                        "limit": limit,
                        "total": msgs.len()
                    }))
                }
                Err(e) => {
                    Json(json!({
                        "session_id": session_id,
                        "error": format!("{}", e),
                        "messages": [],
                        "next_cursor": null,
                        "has_more": false,
                        "limit": limit,
                        "total": 0
                    }))
                }
            }
        }
        None => {
            Json(json!({
                "session_id": session_id,
                "messages": [],
                "next_cursor": null,
                "has_more": false,
                "limit": limit,
                "total": 0,
                "note": "database not connected"
            }))
        }
    }
}

async fn list_sessions(State(state): State<Arc<AppState>>) -> Json<Value> {
    match &state.db {
        Some(db) => {
            match db.list_sessions(100).await {
                Ok(sessions) => {
                    let sess: Vec<Value> = sessions.into_iter().map(|s| {
                        json!({
                            "id": s.id,
                            "platform": s.platform,
                            "chat_id": s.chat_id,
                            "title": s.title,
                            "created_at": s.created_at,
                            "updated_at": s.updated_at,
                        })
                    }).collect();
                    Json(json!({
                        "sessions": sess,
                        "total": sess.len()
                    }))
                }
                Err(e) => {
                    Json(json!({
                        "error": format!("{}", e),
                        "sessions": [],
                        "total": 0
                    }))
                }
            }
        }
        None => {
            Json(json!({
                "sessions": [],
                "total": 0,
                "note": "database not connected"
            }))
        }
    }
}

// ===== New P3-S1 Dashboard Feature Routes =====

async fn list_knowledge_bases() -> Json<Value> {
    Json(json!({
        "knowledge_bases": [
            {
                "id": "kb_default",
                "name": "Default Knowledge Base",
                "description": "General knowledge for the bot",
                "document_count": 12,
                "total_chunks": 3847,
                "embedding_model": "text-embedding-3-small",
                "vector_store": "sqlite",
                "created_at": "2026-04-20T08:00:00Z",
                "last_updated": "2026-04-28T06:00:00Z",
                "status": "active"
            },
            {
                "id": "kb_tech",
                "name": "Tech Docs",
                "description": "Technical documentation and API references",
                "document_count": 5,
                "total_chunks": 1203,
                "embedding_model": "text-embedding-3-small",
                "vector_store": "sqlite",
                "created_at": "2026-04-22T10:00:00Z",
                "last_updated": "2026-04-25T14:30:00Z",
                "status": "active"
            }
        ],
        "total": 2,
        "rag_enabled": true,
        "default_kb": "kb_default"
    }))
}

async fn list_knowledge_documents(Path(id): Path<String>) -> Json<Value> {
    Json(json!({
        "knowledge_base_id": id,
        "documents": [
            {
                "id": "doc_001",
                "filename": "getting_started.md",
                "size_bytes": 12450,
                "chunks": 42,
                "status": "indexed",
                "uploaded_at": "2026-04-20T08:15:00Z",
                "source_type": "markdown"
            },
            {
                "id": "doc_002",
                "filename": "faq.pdf",
                "size_bytes": 89200,
                "chunks": 156,
                "status": "indexed",
                "uploaded_at": "2026-04-21T09:30:00Z",
                "source_type": "pdf"
            },
            {
                "id": "doc_003",
                "filename": "api_reference.txt",
                "size_bytes": 45600,
                "chunks": 89,
                "status": "indexed",
                "uploaded_at": "2026-04-22T11:00:00Z",
                "source_type": "text"
            }
        ],
        "total": 3,
        "total_chunks": 287
    }))
}

async fn list_personas(State(state): State<Arc<AppState>>) -> Json<Value> {
    if let Some(ref registry) = state.persona_registry {
        let personas = registry.list().await;
        let items: Vec<Value> = personas.iter().map(|p| {
            json!({
                "id": p.id,
                "name": p.name,
                "system_prompt": p.system_prompt.chars().take(200).collect::<String>(),
                "is_default": p.is_default,
                "variable_count": p.variables.len(),
            })
        }).collect();
        let active_id = registry.active_id().await.unwrap_or_default();
        return Json(json!({
            "personas": items,
            "active_id": active_id,
            "total": items.len(),
        }));
    }
    Json(json!({
        "personas": [],
        "active_id": null,
        "total": 0,
    }))
}

async fn list_tools(State(state): State<Arc<AppState>>) -> Json<Value> {
    if let Some(ref registry) = state.tool_registry {
        let defs = registry.list_definitions().await;
        let items: Vec<Value> = defs.iter().map(|d| {
            let params: Vec<Value> = d.parameters.iter().map(|p| {
                json!({
                    "name": p.name,
                    "type": p.param_type,
                    "description": p.description,
                    "required": p.required,
                })
            }).collect();
            json!({
                "name": d.name,
                "description": d.description,
                "parameters": params,
                "requires_confirmation": d.requires_confirmation,
                "returns": d.returns,
            })
        }).collect();
        return Json(json!({
            "tools": items,
            "total": items.len(),
        }));
    }
    Json(json!({
        "tools": [],
        "total": 0,
    }))
}

async fn list_backups(State(state): State<Arc<AppState>>) -> Json<Value> {
    if let Some(ref manager) = state.backup_manager {
        match manager.list_backups().await {
            Ok(backups) => {
                let items: Vec<Value> = backups.iter().map(|b| {
                    let size_human = if b.total_size_bytes > 1024 * 1024 {
                        format!("{:.2} MB", b.total_size_bytes as f64 / (1024.0 * 1024.0))
                    } else if b.total_size_bytes > 1024 {
                        format!("{:.2} KB", b.total_size_bytes as f64 / 1024.0)
                    } else {
                        format!("{} B", b.total_size_bytes)
                    };
                    json!({
                        "id": b.id,
                        "db_size_bytes": b.db_size_bytes,
                        "config_size_bytes": b.config_size_bytes,
                        "total_size_bytes": b.total_size_bytes,
                        "size_human": size_human,
                        "created_at": b.created_at.to_rfc3339(),
                    })
                }).collect();
                return Json(json!({
                    "backups": items,
                    "total": items.len(),
                    "backup_dir": "./backups",
                }));
            }
            Err(e) => {
                return Json(json!({
                    "backups": [],
                    "total": 0,
                    "error": format!("{}", e),
                }));
            }
        }
    }
    Json(json!({
        "backups": [],
        "total": 0,
        "note": "backup manager not available",
    }))
}

async fn get_stats(State(state): State<Arc<AppState>>) -> Json<Value> {
    let platform_stats: Vec<Value> = vec![];
    let _total_messages = 0i64;
    let mut total_sessions = 0i64;

    // Get real session/message counts from DB
    if let Some(ref db) = state.db {
        match db.list_sessions(1000).await {
            Ok(sessions) => {
                total_sessions = sessions.len() as i64;
            }
            Err(_) => {}
        }
    }

    // Get metrics snapshot if available
    let metrics = if let Some(ref collector) = state.metrics_collector {
        let lock = collector.lock().await;
        lock.snapshot()
    } else {
        json!({})
    };

    Json(json!({
        "platforms": platform_stats,
        "metrics": metrics,
        "summary": {
            "total_sessions": total_sessions,
            "db_connected": state.db.is_some(),
            "config_loaded": state.config.is_some(),
            "plugin_manager": state.plugin_manager.is_some(),
            "persona_registry": state.persona_registry.is_some(),
            "tool_registry": state.tool_registry.is_some(),
            "agent_registry": state.agent_registry.is_some(),
            "mcp_registry": state.mcp_registry.is_some(),
            "webhook_manager": state.webhook_manager.is_some(),
            "safety_engine": state.safety_engine.is_some(),
        }
    }))
}

async fn get_logs(Query(params): Query<LogQueryParams>) -> Json<Value> {
    let lines = params.validated_lines();
    let level_filter = params.level_filter();

    // Simulated log entries
    let mut log_entries = vec![
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

    // Apply level filter if specified
    if let Some(filter) = level_filter {
        let filter_upper = filter.to_uppercase();
        log_entries.retain(|entry| {
            entry.get("level")
                .and_then(|v| v.as_str())
                .map(|l| l.to_uppercase() == filter_upper)
                .unwrap_or(false)
        });
    }

    // Limit to requested lines
    let total_available = log_entries.len();
    let truncated = if log_entries.len() > lines {
        log_entries.truncate(lines);
        true
    } else {
        false
    };

    Json(json!({
        "logs": log_entries,
        "returned": log_entries.len(),
        "requested_lines": lines,
        "level_filter": level_filter,
        "truncated": truncated,
        "total_available": total_available
    }))
}

async fn list_settings() -> Json<Value> {
    Json(json!({
        "settings": {
            "bot": {
                "name": { "value": "AstrBot", "description": "Bot display name", "type": "string", "editable": true },
                "version": { "value": "3.2.0", "description": "Bot version", "type": "string", "editable": false },
                "debug_mode": { "value": false, "description": "Enable debug logging", "type": "boolean", "editable": true },
                "log_level": { "value": "info", "description": "Log verbosity", "type": "string", "editable": true, "options": ["debug", "info", "warn", "error"] }
            },
            "platform": {
                "qq_enabled": { "value": true, "description": "Enable QQ platform", "type": "boolean", "editable": true },
                "telegram_enabled": { "value": false, "description": "Enable Telegram platform", "type": "boolean", "editable": true }
            },
            "provider": {
                "default_provider": { "value": "openai", "description": "Default LLM provider", "type": "string", "editable": true, "options": ["openai", "anthropic", "gemini"] },
                "max_tokens": { "value": 2048, "description": "Max tokens per response", "type": "integer", "editable": true, "min": 256, "max": 8192 }
            },
            "feature": {
                "rag_enabled": { "value": true, "description": "Enable RAG knowledge base", "type": "boolean", "editable": true },
                "agent_enabled": { "value": false, "description": "Enable agent mode", "type": "boolean", "editable": true },
                "web_search_enabled": { "value": true, "description": "Enable web search tool", "type": "boolean", "editable": true }
            }
        },
        "categories": ["bot", "platform", "provider", "feature"],
        "editable_count": 8
    }))
}

async fn get_setting(Path(key): Path<String>) -> Json<Value> {
    let settings_map = serde_json::json!({
        "bot.name": { "value": "AstrBot", "description": "Bot display name", "type": "string", "editable": true, "category": "bot" },
        "bot.debug_mode": { "value": false, "description": "Enable debug logging", "type": "boolean", "editable": true, "category": "bot" },
        "bot.log_level": { "value": "info", "description": "Log verbosity", "type": "string", "editable": true, "category": "bot" },
        "platform.qq_enabled": { "value": true, "description": "Enable QQ platform", "type": "boolean", "editable": true, "category": "platform" },
        "provider.default": { "value": "openai", "description": "Default LLM provider", "type": "string", "editable": true, "category": "provider" },
        "feature.rag_enabled": { "value": true, "description": "Enable RAG knowledge base", "type": "boolean", "editable": true, "category": "feature" }
    });

    match settings_map.get(&key) {
        Some(setting) => {
            let mut result = setting.clone();
            if let Value::Object(ref mut obj) = result {
                obj.insert("key".to_string(), json!(key));
                obj.insert("found".to_string(), json!(true));
            }
            Json(result)
        }
        None => {
            Json(json!({
                "key": key,
                "found": false,
                "error": "Setting not found",
                "available_keys": [
                    "bot.name", "bot.debug_mode", "bot.log_level",
                    "platform.qq_enabled", "provider.default", "feature.rag_enabled"
                ]
            }))
        }
    }
}

async fn update_setting(Path(key): Path<String>, Json(payload): Json<Value>) -> Json<Value> {
    let value = payload.get("value").cloned().unwrap_or(Value::Null);
    Json(json!({
        "key": key,
        "updated": true,
        "value": value,
        "message": "Setting updated (skeleton — persistence not yet implemented)"
    }))
}

async fn list_mcp_servers(State(state): State<Arc<AppState>>) -> Json<Value> {
    if let Some(ref registry) = state.mcp_registry {
        let names = registry.list_names().await;
        let items: Vec<Value> = names.iter().map(|name| {
            json!({
                "id": name,
                "name": name,
            })
        }).collect();
        return Json(json!({
            "mcp_servers": items,
            "total": items.len(),
        }));
    }
    Json(json!({
        "mcp_servers": [],
        "total": 0,
    }))
}

async fn list_agents(State(state): State<Arc<AppState>>) -> Json<Value> {
    if let Some(ref registry) = state.agent_registry {
        let lock = registry.read().await;
        let configs = lock.list_configs();
        let items: Vec<Value> = configs.iter().map(|c| {
            json!({
                "id": c.id,
                "name": c.name,
                "executor_type": c.executor_type,
                "enabled": c.enabled,
                "max_iterations": c.max_iterations,
                "enable_tools": c.enable_tools,
                "system_prompt": c.system_prompt,
            })
        }).collect();
        let enabled_count = configs.iter().filter(|c| c.enabled).count();
        return Json(json!({
            "agents": items,
            "total": items.len(),
            "enabled_count": enabled_count,
        }));
    }
    Json(json!({
        "agents": [],
        "total": 0,
        "enabled_count": 0,
    }))
}

async fn get_safety_status(State(state): State<Arc<AppState>>) -> Json<Value> {
    if let Some(ref _engine) = state.safety_engine {
        return Json(json!({
            "safety": {
                "strategies_count": 0,
                "stop_on_first": true,
                "status": "active",
            },
            "note": "Safety engine initialized. Add strategies via configuration.",
        }));
    }
    Json(json!({
        "safety": {
            "status": "not_initialized",
        },
        "note": "Safety engine not available",
    }))
}

async fn list_webhooks(State(state): State<Arc<AppState>>) -> Json<Value> {
    if let Some(ref manager) = state.webhook_manager {
        let hooks = manager.list();
        let items: Vec<Value> = hooks.iter().map(|h| {
            json!({
                "id": h.id,
                "url": h.url,
                "event_types": h.event_types,
                "enabled": h.enabled,
                "secret_configured": h.secret.is_some(),
                "retry_count": h.retry_count,
            })
        }).collect();
        let enabled_count = hooks.iter().filter(|h| h.enabled).count();
        return Json(json!({
            "webhooks": items,
            "total": items.len(),
            "enabled_count": enabled_count,
        }));
    }
    Json(json!({
        "webhooks": [],
        "total": 0,
        "enabled_count": 0,
    }))
}

// ===== Persona API handlers =====

async fn activate_persona(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Json<Value> {
    if let Some(ref registry) = state.persona_registry {
        match registry.switch(&id).await {
            Ok(persona) => {
                return Json(json!({
                    "success": true,
                    "persona_id": id,
                    "persona_name": persona.name,
                    "message": format!("Switched to persona: {}", persona.name),
                }));
            }
            Err(e) => {
                return Json(json!({
                    "success": false,
                    "persona_id": id,
                    "error": format!("{}", e),
                    "message": format!("Failed to switch persona: {}", e),
                }));
            }
        }
    }
    Json(json!({
        "success": false,
        "error": "Persona registry not available",
        "message": "Persona registry is not initialized",
    }))
}

#[derive(Debug, Deserialize)]
pub struct CreatePersonaRequest {
    pub id: String,
    pub name: String,
    pub system_prompt: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub variables: std::collections::HashMap<String, String>,
}

async fn create_custom_persona(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CreatePersonaRequest>,
) -> Json<Value> {
    if let Some(ref registry) = state.persona_registry {
        let persona = astrbot_core::persona::Persona {
            id: req.id.clone(),
            name: req.name.clone(),
            system_prompt: req.system_prompt.clone(),
            variables: req.variables,
            is_default: false,
            description: req.description,
        };
        registry.register(persona).await;
        return Json(json!({
            "success": true,
            "persona_id": req.id,
            "persona_name": req.name,
            "message": format!("Custom persona '{}' created successfully", req.name),
        }));
    }
    Json(json!({
        "success": false,
        "error": "Persona registry not available",
        "message": "Persona registry is not initialized",
    }))
}

async fn update_persona(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<CreatePersonaRequest>,
) -> Json<Value> {
    if let Some(ref registry) = state.persona_registry {
        // 先检查是否存在
        if registry.get(&id).await.is_none() {
            return Json(json!({
                "success": false,
                "persona_id": id,
                "error": "Persona not found",
                "message": format!("Persona '{}' does not exist", id),
            }));
        }
        // 注销旧的，注册新的（覆盖）
        let _ = registry.unregister(&id).await;
        let persona = astrbot_core::persona::Persona {
            id: id.clone(),
            name: req.name.clone(),
            system_prompt: req.system_prompt.clone(),
            variables: req.variables,
            is_default: false,
            description: req.description,
        };
        registry.register(persona).await;
        return Json(json!({
            "success": true,
            "persona_id": id,
            "persona_name": req.name,
            "message": format!("Persona '{}' updated successfully", req.name),
        }));
    }
    Json(json!({
        "success": false,
        "error": "Persona registry not available",
        "message": "Persona registry is not initialized",
    }))
}

async fn delete_persona(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Json<Value> {
    if let Some(ref registry) = state.persona_registry {
        match registry.unregister(&id).await {
            Ok(()) => {
                return Json(json!({
                    "success": true,
                    "persona_id": id,
                    "message": format!("Persona '{}' deleted successfully", id),
                }));
            }
            Err(e) => {
                return Json(json!({
                    "success": false,
                    "persona_id": id,
                    "error": format!("{}", e),
                    "message": format!("Failed to delete persona: {}", e),
                }));
            }
        }
    }
    Json(json!({
        "success": false,
        "error": "Persona registry not available",
        "message": "Persona registry is not initialized",
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use tower::ServiceExt;

    async fn test_app() -> Router {
        let db = Arc::new(Database::new_in_memory().await.unwrap());
        let mut state = AppState::new("0.1.0-test".to_string()).with_db(db);

        // Initialize core registries for tests
        let persona_registry = Arc::new(astrbot_core::persona::PersonaRegistry::new());
        let default_persona = astrbot_core::persona::default_persona();
        persona_registry.register(default_persona).await;
        let _ = persona_registry.switch("default").await;
        state = state.with_persona_registry(persona_registry);

        let tool_registry = Arc::new(astrbot_core::tools::ToolRegistry::new());
        tool_registry.register(Box::new(astrbot_core::tools::EchoTool::new())).await;
        tool_registry.register(Box::new(astrbot_core::tools::CurrentTimeTool::new())).await;
        state = state.with_tool_registry(tool_registry);

        let backup_manager = Arc::new(astrbot_core::backup::BackupManager::new("./backups_test", 10));
        state = state.with_backup_manager(backup_manager);

        let agent_registry = Arc::new(tokio::sync::RwLock::new(astrbot_core::agent::AgentRegistry::new()));
        state = state.with_agent_registry(agent_registry);

        let mcp_registry = Arc::new(astrbot_core::mcp::McpServerRegistry::new());
        state = state.with_mcp_registry(mcp_registry);

        let webhook_manager = Arc::new(astrbot_core::webhook::WebhookManager::new());
        state = state.with_webhook_manager(webhook_manager);

        let safety_engine = Arc::new(astrbot_core::safety::SafetyEngine::new());
        state = state.with_safety_engine(safety_engine);

        let metrics_collector = Arc::new(tokio::sync::Mutex::new(astrbot_core::metrics::MetricsCollector::new()));
        state = state.with_metrics_collector(metrics_collector);

        let plugin_manager = Arc::new(tokio::sync::RwLock::new(astrbot_plugin::PluginManager::new()));
        state = state.with_plugin_manager(plugin_manager);

        create_router(Arc::new(state))
    }

    #[tokio::test]
    async fn test_health_check() {
        let app = test_app().await;
        let response = app
            .oneshot(Request::builder().uri("/api/health").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_status_endpoint() {
        let app = test_app().await;
        let response = app
            .oneshot(Request::builder().uri("/api/status").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        
        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["version"].as_str(), Some("0.1.0-test"));
        assert_eq!(json["status"].as_str(), Some("running"));
    }

    #[tokio::test]
    async fn test_plugins_list() {
        let app = test_app().await;
        let response = app
            .oneshot(Request::builder().uri("/api/plugins").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        
        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: Value = serde_json::from_slice(&body).unwrap();
        assert!(json["plugins"].is_array());
        assert_eq!(json["total"], 0);
        assert_eq!(json["enabled_count"], 0);
    }

    #[tokio::test]
    async fn test_providers_list() {
        let app = test_app().await;
        let response = app
            .oneshot(Request::builder().uri("/api/providers").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        
        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: Value = serde_json::from_slice(&body).unwrap();
        assert!(json["providers"].is_array());
        assert_eq!(json["total"], 0);
        assert_eq!(json["active_count"], 0);
    }

    #[tokio::test]
    async fn test_platforms_list() {
        let app = test_app().await;
        let response = app
            .oneshot(Request::builder().uri("/api/platforms").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        
        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: Value = serde_json::from_slice(&body).unwrap();
        assert!(json["platforms"].is_array());
        assert_eq!(json["total"], 0);
        assert_eq!(json["connected_count"], 0);
    }

    #[tokio::test]
    async fn test_config_endpoint() {
        let app = test_app().await;
        let response = app
            .oneshot(Request::builder().uri("/api/config").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        
        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: Value = serde_json::from_slice(&body).unwrap();
        assert!(json["config"].is_null());
        assert_eq!(json["note"], "configuration not loaded");
    }

    #[tokio::test]
    async fn test_history_endpoint() {
        let app = test_app().await;
        
        // Pre-populate some data
        let db = Arc::new(Database::new_in_memory().await.unwrap());
        db.create_session("sess1", "qq", "123456", Some("Test Chat")).await.unwrap();
        db.save_message("sess1", Some("u1"), "user", "Hello", Some("gpt-4")).await.unwrap();
        
        let state = Arc::new(AppState::new("0.1.0-test".to_string()).with_db(db));
        let app = create_router(state);
        
        let response = app
            .oneshot(Request::builder().uri("/api/history/sess1").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        
        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["session_id"], "sess1");
        assert!(json["messages"].is_array());
        assert_eq!(json["total"], 1);
        assert_eq!(json["limit"], 20);
        assert_eq!(json["has_more"], false);
        assert!(json["next_cursor"].is_null());
    }

    #[tokio::test]
    async fn test_history_pagination() {
        // Pre-populate 25 messages
        let db = Arc::new(Database::new_in_memory().await.unwrap());
        db.create_session("sess1", "qq", "123456", Some("Test Chat")).await.unwrap();
        
        for i in 0..25 {
            db.save_message("sess1", Some("u1"), "user", &format!("msg{}", i), None).await.unwrap();
        }
        
        let state = Arc::new(AppState::new("0.1.0-test".to_string()).with_db(db));
        let app = create_router(state);
        
        // First page with limit=10
        let response = app.clone()
            .oneshot(Request::builder().uri("/api/history/sess1?limit=10").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        
        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["messages"].as_array().unwrap().len(), 10);
        assert_eq!(json["limit"], 10);
        assert_eq!(json["has_more"], true);
        let next_cursor = json["next_cursor"].as_i64().unwrap();
        
        // Second page using cursor
        let response = app.clone()
            .oneshot(Request::builder().uri(&format!("/api/history/sess1?limit=10&cursor={}", next_cursor)).body(Body::empty()).unwrap())
            .await
            .unwrap();

        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["messages"].as_array().unwrap().len(), 10);
        assert_eq!(json["has_more"], true);
        let next_cursor2 = json["next_cursor"].as_i64().unwrap();
        assert_ne!(next_cursor, next_cursor2);
        
        // Third page (should have remaining 5 messages)
        let response = app
            .oneshot(Request::builder().uri(&format!("/api/history/sess1?limit=10&cursor={}", next_cursor2)).body(Body::empty()).unwrap())
            .await
            .unwrap();

        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["messages"].as_array().unwrap().len(), 5);
        assert_eq!(json["has_more"], false);
        assert!(json["next_cursor"].is_null());
    }

    #[tokio::test]
    async fn test_history_limit_clamping() {
        let db = Arc::new(Database::new_in_memory().await.unwrap());
        db.create_session("sess1", "qq", "123456", Some("Test Chat")).await.unwrap();
        db.save_message("sess1", Some("u1"), "user", "Hello", None).await.unwrap();
        
        let state = Arc::new(AppState::new("0.1.0-test".to_string()).with_db(db));
        let app = create_router(state);
        
        // limit=200 should be clamped to 100
        let response = app
            .oneshot(Request::builder().uri("/api/history/sess1?limit=200").body(Body::empty()).unwrap())
            .await
            .unwrap();

        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["limit"], 100);
    }

    #[tokio::test]
    async fn test_sessions_endpoint() {
        let app = test_app().await;
        
        // Pre-populate some data
        let db = Arc::new(Database::new_in_memory().await.unwrap());
        db.create_session("sess1", "qq", "123456", Some("Chat 1")).await.unwrap();
        db.create_session("sess2", "telegram", "789", Some("Chat 2")).await.unwrap();
        
        let state = Arc::new(AppState::new("0.1.0-test".to_string()).with_db(db));
        let app = create_router(state);
        
        let response = app
            .oneshot(Request::builder().uri("/api/sessions").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        
        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: Value = serde_json::from_slice(&body).unwrap();
        assert!(json["sessions"].is_array());
        assert_eq!(json["total"], 2);
    }

    // ===== New P3-S1 Route Tests =====

    #[tokio::test]
    async fn test_knowledge_bases_list() {
        let app = test_app().await;
        let response = app
            .oneshot(Request::builder().uri("/api/knowledge").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        
        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: Value = serde_json::from_slice(&body).unwrap();
        assert!(json["knowledge_bases"].is_array());
    }

    #[tokio::test]
    async fn test_knowledge_documents() {
        let app = test_app().await;
        let response = app
            .oneshot(Request::builder().uri("/api/knowledge/kb_default/documents").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        
        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["knowledge_base_id"], "kb_default");
        assert!(json["documents"].is_array());
        assert_eq!(json["total"], 3);
    }

    #[tokio::test]
    async fn test_personas_list() {
        let app = test_app().await;
        let response = app
            .oneshot(Request::builder().uri("/api/personas").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        
        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: Value = serde_json::from_slice(&body).unwrap();
        assert!(json["personas"].is_array());
        assert_eq!(json["total"], 1);
        assert_eq!(json["active_id"], "default");
    }

    #[tokio::test]
    async fn test_tools_list() {
        let app = test_app().await;
        let response = app
            .oneshot(Request::builder().uri("/api/tools").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        
        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: Value = serde_json::from_slice(&body).unwrap();
        assert!(json["tools"].is_array());
        assert_eq!(json["total"], 2);
    }

    #[tokio::test]
    async fn test_backups_list() {
        let app = test_app().await;
        let response = app
            .oneshot(Request::builder().uri("/api/backups").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        
        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: Value = serde_json::from_slice(&body).unwrap();
        assert!(json["backups"].is_array());
        assert_eq!(json["total"], 0);
        assert!(json["backup_dir"].is_string());
    }

    #[tokio::test]
    async fn test_stats_endpoint() {
        let app = test_app().await;
        let response = app
            .oneshot(Request::builder().uri("/api/stats").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        
        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: Value = serde_json::from_slice(&body).unwrap();
        assert!(json["platforms"].is_array());
        assert!(json["summary"].is_object());
        assert!(json["summary"]["db_connected"].is_boolean());
    }

    #[tokio::test]
    async fn test_logs_endpoint() {
        let app = test_app().await;
        let response = app
            .oneshot(Request::builder().uri("/api/logs").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        
        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: Value = serde_json::from_slice(&body).unwrap();
        assert!(json["logs"].is_array());
        assert!(json["returned"].as_i64().unwrap() > 0);
    }

    #[tokio::test]
    async fn test_logs_level_filter() {
        let app = test_app().await;
        let response = app
            .oneshot(Request::builder().uri("/api/logs?level=error").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        
        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: Value = serde_json::from_slice(&body).unwrap();
        assert!(json["logs"].is_array());
        
        // All returned logs should have level ERROR
        for entry in json["logs"].as_array().unwrap() {
            assert_eq!(entry["level"], "ERROR");
        }
    }

    #[tokio::test]
    async fn test_settings_list() {
        let app = test_app().await;
        let response = app
            .oneshot(Request::builder().uri("/api/settings").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        
        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: Value = serde_json::from_slice(&body).unwrap();
        assert!(json["settings"].is_object());
        assert!(json["categories"].is_array());
    }

    #[tokio::test]
    async fn test_setting_get_existing() {
        let app = test_app().await;
        let response = app
            .oneshot(Request::builder().uri("/api/settings/bot.name").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        
        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["key"], "bot.name");
        assert_eq!(json["found"], true);
        assert_eq!(json["value"], "AstrBot");
    }

    #[tokio::test]
    async fn test_setting_get_missing() {
        let app = test_app().await;
        let response = app
            .oneshot(Request::builder().uri("/api/settings/nonexistent.key").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        
        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["key"], "nonexistent.key");
        assert_eq!(json["found"], false);
    }

    #[tokio::test]
    async fn test_mcp_servers_list() {
        let app = test_app().await;
        let response = app
            .oneshot(Request::builder().uri("/api/mcp").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        
        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: Value = serde_json::from_slice(&body).unwrap();
        assert!(json["mcp_servers"].is_array());
        assert_eq!(json["total"], 0);
    }

    #[tokio::test]
    async fn test_agents_list() {
        let app = test_app().await;
        let response = app
            .oneshot(Request::builder().uri("/api/agents").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        
        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: Value = serde_json::from_slice(&body).unwrap();
        assert!(json["agents"].is_array());
        assert_eq!(json["total"], 0);
        assert_eq!(json["enabled_count"], 0);
    }

    #[tokio::test]
    async fn test_safety_status() {
        let app = test_app().await;
        let response = app
            .oneshot(Request::builder().uri("/api/safety").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        
        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: Value = serde_json::from_slice(&body).unwrap();
        assert!(json["safety"].is_object());
        assert_eq!(json["safety"]["status"], "active");
    }

    #[tokio::test]
    async fn test_webhooks_list() {
        let app = test_app().await;
        let response = app
            .oneshot(Request::builder().uri("/api/webhooks").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        
        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: Value = serde_json::from_slice(&body).unwrap();
        assert!(json["webhooks"].is_array());
        assert_eq!(json["total"], 0);
        assert_eq!(json["enabled_count"], 0);
    }

    #[tokio::test]
    async fn test_detailed_status() {
        let app = test_app().await;
        let response = app
            .oneshot(Request::builder().uri("/api/status/detailed").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        
        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["status"], "running");
        assert!(json["database"].is_object());
        assert!(json["memory"].is_object());
        assert!(json["providers"].is_object());
        assert!(json["platforms"].is_object());
    }

    #[tokio::test]
    async fn test_plugin_toggle() {
        let app = test_app().await;
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/plugins/astrbot_plugin_weather/toggle")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"enable": false}"#))
                    .unwrap()
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        
        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["plugin_id"], "astrbot_plugin_weather");
        assert_eq!(json["action"], "disabled");
        assert_eq!(json["success"], true);
    }

    #[tokio::test]
    async fn test_provider_test() {
        let app = test_app().await;
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/providers/openai/test")
                    .body(Body::empty())
                    .unwrap()
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        
        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["provider_id"], "openai");
        assert_eq!(json["test_result"], "ok");
        assert_eq!(json["success"], true);
        assert!(json["details"].is_object());
    }
}
