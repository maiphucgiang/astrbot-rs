//! Knowledge Base REST API handlers — wrap KbTools as axum handlers.

use axum::extract::{Path, State};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;

use crate::app_state::AppState;
use astrbot_core::tools::Tool;

#[derive(Debug, Deserialize)]
pub struct SearchKbRequest {
    pub query: String,
    #[serde(default = "default_collection")]
    pub collection: String,
    #[serde(default = "default_top_k")]
    pub top_k: usize,
}

#[derive(Debug, Deserialize)]
pub struct IndexKbRequest {
    pub doc_id: String,
    pub title: String,
    pub content: String,
    #[serde(default = "default_chunk_size")]
    pub chunk_size: usize,
    #[serde(default = "default_overlap")]
    pub overlap: usize,
    #[serde(default = "default_collection")]
    pub collection: String,
}

fn default_collection() -> String {
    "default".to_string()
}
fn default_top_k() -> usize {
    5
}
fn default_chunk_size() -> usize {
    500
}
fn default_overlap() -> usize {
    50
}

pub async fn list_kb_collections(State(state): State<AppState>) -> Json<Value> {
    if let Some(ref km) = state.kb_manager {
        let collections = km.list_kbs();
        return Json(json!({
            "collections": collections,
            "total": collections.len(),
        }));
    }
    Json(json!({ "collections": [], "total": 0, "note": "kb_manager not available" }))
}

pub async fn search_kb(
    State(state): State<AppState>,
    Json(req): Json<SearchKbRequest>,
) -> Json<Value> {
    if let Some(ref km) = state.kb_manager {
        let tools = match km.get_kb_tools(&req.collection) {
            Some(tools) => tools,
            None => {
                return Json(json!({
                    "error": format!("Knowledge base '{}' not found", req.collection),
                    "query": req.query,
                }));
            }
        };
        let (search, _, _) = tools;
        match search.execute(&json!({ "query": req.query })).await {
            Ok(astrbot_core::tools::ToolResult::Success { output }) => {
                return Json(json!({ "results": output, "query": req.query }));
            }
            Ok(astrbot_core::tools::ToolResult::Error { message }) => {
                return Json(json!({ "error": message, "query": req.query }));
            }
            _ => {
                return Json(json!({ "error": "unexpected tool result", "query": req.query }));
            }
        }
    }
    Json(
        json!({ "results": [], "total": 0, "note": "kb_manager not available", "query": req.query }),
    )
}

pub async fn index_kb(
    State(state): State<AppState>,
    Json(req): Json<IndexKbRequest>,
) -> Json<Value> {
    if let Some(ref km) = state.kb_manager {
        let tools = match km.get_kb_tools(&req.collection) {
            Some(tools) => tools,
            None => {
                return Json(json!({
                    "success": false,
                    "error": format!("Knowledge base '{}' not found", req.collection),
                }));
            }
        };
        let (_, index, _) = tools;
        match index
            .execute(&json!({
                "doc_id": req.doc_id,
                "title": req.title,
                "content": req.content,
                "chunk_size": req.chunk_size,
                "overlap": req.overlap,
            }))
            .await
        {
            Ok(astrbot_core::tools::ToolResult::Success { output }) => {
                return Json(json!({ "success": true, "doc_id": req.doc_id, "result": output }));
            }
            Ok(astrbot_core::tools::ToolResult::Error { message }) => {
                return Json(json!({ "success": false, "error": message }));
            }
            _ => {
                return Json(json!({ "success": false, "error": "unexpected tool result" }));
            }
        }
    }
    Json(json!({ "success": false, "error": "kb_manager not available" }))
}

pub async fn delete_kb_doc(State(state): State<AppState>, Path(id): Path<String>) -> Json<Value> {
    if let Some(ref km) = state.kb_manager {
        let kb_names: Vec<String> = km.list_kbs();
        for kb_name in &kb_names {
            if let Some(tools) = km.get_kb_tools(kb_name) {
                let (_, _, delete) = tools;
                match delete.execute(&json!({ "doc_id": id })).await {
                    Ok(astrbot_core::tools::ToolResult::Success { output }) => {
                        let removed = output
                            .get("chunks_removed")
                            .and_then(|v| v.as_u64())
                            .unwrap_or(0);
                        if removed > 0 {
                            return Json(json!({
                                "success": true,
                                "doc_id": id,
                                "chunks_removed": removed,
                                "knowledge_base": kb_name,
                            }));
                        }
                    }
                    _ => continue,
                }
            }
        }
        return Json(json!({
            "success": false,
            "doc_id": id,
            "error": "Document not found in any knowledge base",
        }));
    }
    Json(json!({ "success": false, "error": "kb_manager not available" }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_kb_list_collections_handler_stub() {
        let state = AppState::new(
            std::sync::Arc::new(tokio::sync::RwLock::new(
                astrbot_plugin::PluginManager::new(std::path::PathBuf::from("plugins")),
            )),
            std::sync::Arc::new(tokio::sync::RwLock::new(
                astrbot_provider::client::ProviderManager::new(),
            )),
        );
        let result = list_kb_collections(State(state)).await;
        let json = result.0;
        assert!(json["collections"].is_array());
        assert_eq!(json["total"], 0);
    }

    #[tokio::test]
    async fn test_kb_index_stub() {
        let state = AppState::new(
            std::sync::Arc::new(tokio::sync::RwLock::new(
                astrbot_plugin::PluginManager::new(std::path::PathBuf::from("plugins")),
            )),
            std::sync::Arc::new(tokio::sync::RwLock::new(
                astrbot_provider::client::ProviderManager::new(),
            )),
        );
        let req = IndexKbRequest {
            doc_id: "doc_test".to_string(),
            title: "Test Doc".to_string(),
            content: "Rust memory safety".to_string(),
            chunk_size: 30,
            overlap: 5,
            collection: "default".to_string(),
        };
        let result = index_kb(State(state), Json(req)).await;
        let json = result.0;
        assert_eq!(json["success"], false);
        assert_eq!(json["error"], "kb_manager not available");
    }

    #[tokio::test]
    async fn test_kb_delete_stub() {
        let state = AppState::new(
            std::sync::Arc::new(tokio::sync::RwLock::new(
                astrbot_plugin::PluginManager::new(std::path::PathBuf::from("plugins")),
            )),
            std::sync::Arc::new(tokio::sync::RwLock::new(
                astrbot_provider::client::ProviderManager::new(),
            )),
        );
        let result = delete_kb_doc(State(state), Path("del_me".to_string())).await;
        let json = result.0;
        assert_eq!(json["success"], false);
    }
}
