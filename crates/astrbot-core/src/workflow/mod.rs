use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A node in a workflow graph.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum WorkflowNode {
    LlmCall {
        id: String,
        prompt: String,
        model: Option<String>,
    },
    ToolCall {
        id: String,
        tool_name: String,
        arguments: serde_json::Value,
    },
    Conditional {
        id: String,
        condition: String,
    },
    End {
        id: String,
        output_key: Option<String>,
    },
}

/// A directed graph representing a workflow.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WorkflowGraph {
    nodes: Vec<WorkflowNode>,
    edges: Vec<(String, String)>,
}

impl WorkflowGraph {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_node(&mut self, node: WorkflowNode) {
        self.nodes.push(node);
    }

    pub fn add_edge(&mut self, from: &str, to: &str) {
        self.edges.push((from.to_string(), to.to_string()));
    }

    pub fn nodes(&self) -> &Vec<WorkflowNode> {
        &self.nodes
    }

    pub fn edges(&self) -> &Vec<(String, String)> {
        &self.edges
    }

    /// Execute the workflow and return final state.
    pub async fn execute(&self) -> WorkflowState {
        let mut variables: HashMap<String, String> = HashMap::new();

        for node in &self.nodes {
            let node_id = match node {
                WorkflowNode::LlmCall { id, prompt, .. } => {
                    variables.insert(id.clone(), prompt.clone());
                    id.clone()
                }
                WorkflowNode::ToolCall { id, tool_name, .. } => {
                    variables.insert(id.clone(), format!("tool: {}", tool_name));
                    id.clone()
                }
                WorkflowNode::Conditional { id, .. } => {
                    variables.insert(id.clone(), "condition".to_string());
                    id.clone()
                }
                WorkflowNode::End { id, output_key } => {
                    if let Some(key) = output_key {
                        variables.insert(id.clone(), format!("output: {}", key));
                    }
                    id.clone()
                }
            };
            let _ = node_id;
        }

        WorkflowState {
            variables,
            completed: true,
            current_node: None,
        }
    }
}

/// The runtime state of a workflow execution.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WorkflowState {
    pub variables: HashMap<String, String>,
    pub completed: bool,
    pub current_node: Option<String>,
}
