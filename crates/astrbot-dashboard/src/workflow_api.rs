use astrbot_core::workflow::{WorkflowGraph, WorkflowNode, WorkflowState};
use axum::{
    extract::State,
    routing::{get, post},
    Json, Router,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;
use tokio::sync::RwLock;

/// A stored workflow definition with metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowDefinition {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub graph: WorkflowGraph,
    pub created_at: String,
}

/// In-memory workflow registry.
#[derive(Debug, Clone, Default)]
pub struct WorkflowRegistry {
    workflows: Arc<RwLock<Vec<WorkflowDefinition>>>,
}

impl WorkflowRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn add(&self, workflow: WorkflowDefinition) {
        let mut lock = self.workflows.write().await;
        lock.push(workflow);
    }

    pub async fn list(&self) -> Vec<Value> {
        let lock = self.workflows.read().await;
        lock.iter()
            .map(|w| {
                json!({
                    "id": w.id,
                    "name": w.name,
                    "description": w.description,
                    "node_count": w.graph.nodes().len(),
                    "edge_count": w.graph.edges().len(),
                    "created_at": w.created_at,
                })
            })
            .collect()
    }

    pub async fn get(&self, id: &str) -> Option<WorkflowDefinition> {
        let lock = self.workflows.read().await;
        lock.iter().find(|w| w.id == id).cloned()
    }

    pub async fn execute(&self, id: &str) -> Option<WorkflowState> {
        let workflow = self.get(id).await?;
        Some(workflow.graph.execute().await)
    }

    pub async fn seed_sample(&self) {
        let mut graph = WorkflowGraph::new();
        graph.add_node(WorkflowNode::LlmCall {
            id: "ask".to_string(),
            prompt: "What is the weather today?".to_string(),
            model: Some("gpt-4".to_string()),
        });
        graph.add_node(WorkflowNode::ToolCall {
            id: "search".to_string(),
            tool_name: "web_search".to_string(),
            arguments: serde_json::json!({"query": "weather"}),
        });
        graph.add_node(WorkflowNode::End {
            id: "end".to_string(),
            output_key: Some("ask_output".to_string()),
        });
        graph.add_edge("ask", "search");
        graph.add_edge("search", "end");

        let sample = WorkflowDefinition {
            id: "sample-weather".to_string(),
            name: "Weather Query".to_string(),
            description: Some("Ask LLM then search web for weather.".to_string()),
            graph,
            created_at: chrono::Utc::now().to_rfc3339(),
        };

        self.add(sample).await;
    }
}

#[derive(Debug, Deserialize)]
pub struct ExecuteWorkflowRequest {
    pub workflow_id: String,
}

async fn list_workflows(State(state): State<crate::app_state::AppState>) -> Json<Value> {
    if let Some(ref registry) = state.workflow_registry {
        let workflows = registry.list().await;
        Json(json!({ "workflows": workflows }))
    } else {
        Json(json!({ "workflows": [] }))
    }
}

async fn execute_workflow(
    State(state): State<crate::app_state::AppState>,
    Json(req): Json<ExecuteWorkflowRequest>,
) -> Json<Value> {
    if let Some(ref registry) = state.workflow_registry {
        match registry.execute(&req.workflow_id).await {
            Some(state) => {
                let variables: Value = serde_json::to_value(&state.variables).unwrap_or_default();
                Json(json!({
                    "success": true,
                    "workflow_id": req.workflow_id,
                    "completed": state.completed,
                    "current_node": state.current_node,
                    "variables": variables,
                }))
            }
            None => Json(json!({
                "success": false,
                "error": format!("workflow '{}' not found", req.workflow_id),
            })),
        }
    } else {
        Json(json!({
            "success": false,
            "error": "workflow registry not initialized",
        }))
    }
}

pub fn create_workflow_router() -> Router<crate::app_state::AppState> {
    Router::new()
        .route("/api/workflows/list", get(list_workflows))
        .route("/api/workflows/execute", post(execute_workflow))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_workflow_registry_list_and_execute() {
        let registry = Arc::new(WorkflowRegistry::new());
        registry.seed_sample().await;

        let list = registry.list().await;
        assert_eq!(list.len(), 1);
        assert_eq!(list[0]["id"], "sample-weather");
        assert_eq!(list[0]["name"], "Weather Query");
        assert_eq!(list[0]["node_count"], 3);
        assert_eq!(list[0]["edge_count"], 2);

        let state = registry.execute("sample-weather").await.unwrap();
        assert!(state.completed);
        assert_eq!(state.current_node, None);
        assert!(state.variables.contains_key("ask"));
        assert!(state.variables.contains_key("search"));
        assert!(state.variables.contains_key("end"));
    }

    #[tokio::test]
    async fn test_workflow_execute_not_found() {
        let registry = Arc::new(WorkflowRegistry::new());
        let result = registry.execute("non-existent").await;
        assert!(result.is_none());
    }
}
