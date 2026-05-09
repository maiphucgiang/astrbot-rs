use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::errors::Result;
use crate::message::MessageChain;
use crate::platform::MessageSource;
use crate::provider::{ChatConfig, ChatMessage, Provider};

mod executors;
pub use executors::*;

mod tool_loop;
pub use tool_loop::*;

mod coze;
pub use coze::*;

mod dify;
pub use dify::*;

mod dashscope;
pub use dashscope::*;

mod deerflow;
pub use deerflow::*;

pub mod context;

/// Agent execution result
#[derive(Debug, Clone)]
pub enum AgentResult {
    /// Direct text response
    Text { content: String },
    /// Tool call request (function calling)
    ToolCall { calls: Vec<ToolCall> },
    /// No action / pass through to LLM
    PassThrough,
    /// Error
    Error { message: String },
}

/// A tool call requested by an agent
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
}

/// Tool execution result to feed back to agent
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    pub call_id: String,
    pub success: bool,
    pub output: serde_json::Value,
    pub error: Option<String>,
}

/// Context for agent execution
#[derive(Clone)]
pub struct AgentContext {
    /// Current conversation messages
    pub messages: Vec<ChatMessage>,
    /// Message source (for replies)
    pub source: MessageSource,
    /// User ID
    pub user_id: String,
    /// Session ID
    pub session_id: String,
    /// Extra context data
    pub extras: HashMap<String, serde_json::Value>,
}

impl AgentContext {
    pub fn new(source: MessageSource, user_id: String, session_id: String) -> Self {
        Self {
            messages: vec![],
            source,
            user_id,
            session_id,
            extras: HashMap::new(),
        }
    }
}

/// Base trait for all agent executors
#[async_trait]
pub trait AgentExecutor: Send + Sync {
    /// Executor name
    fn name(&self) -> &str;
    /// Executor type (coze, dify, deerflow, dashscope, etc.)
    fn executor_type(&self) -> &str;
    /// Initialize the executor
    async fn initialize(&mut self, config: serde_json::Value) -> Result<()>;
    /// Execute with context
    async fn execute(&self, ctx: &AgentContext) -> Result<AgentResult>;
    /// Handle tool results and continue execution
    async fn execute_with_tools(
        &self,
        ctx: &AgentContext,
        tool_results: Vec<ToolResult>,
    ) -> Result<AgentResult> {
        // Default: just re-execute without tools
        let _ = tool_results;
        self.execute(ctx).await
    }
    /// Health check
    async fn health_check(&self) -> Result<bool>;
}

/// Agent configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfig {
    pub id: String,
    pub name: String,
    pub executor_type: String,
    pub enabled: bool,
    /// Executor-specific configuration
    pub config: serde_json::Value,
    /// System prompt override
    pub system_prompt: Option<String>,
    /// Whether to enable tool calling
    pub enable_tools: bool,
    /// Max iterations for agent loop
    pub max_iterations: usize,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            id: "default".to_string(),
            name: "Default Agent".to_string(),
            executor_type: "direct".to_string(),
            enabled: true,
            config: serde_json::Value::Object(serde_json::Map::new()),
            system_prompt: None,
            enable_tools: false,
            max_iterations: 5,
        }
    }
}

/// Registry of agent executors
pub struct AgentRegistry {
    executors: HashMap<String, Box<dyn AgentExecutor>>,
    configs: HashMap<String, AgentConfig>,
}

impl Default for AgentRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl AgentRegistry {
    pub fn new() -> Self {
        Self {
            executors: HashMap::new(),
            configs: HashMap::new(),
        }
    }

    /// Register an executor
    pub fn register(&mut self, config: AgentConfig, executor: Box<dyn AgentExecutor>) {
        let id = config.id.clone();
        self.configs.insert(id.clone(), config);
        self.executors.insert(id, executor);
    }

    /// Get executor by ID
    pub fn get(&self, id: &str) -> Option<&dyn AgentExecutor> {
        self.executors.get(id).map(|e| e.as_ref())
    }

    /// Get config by ID
    pub fn get_config(&self, id: &str) -> Option<&AgentConfig> {
        self.configs.get(id)
    }

    /// List all configs
    pub fn list_configs(&self) -> Vec<&AgentConfig> {
        self.configs.values().collect()
    }

    /// Execute with a specific agent (single turn)
    pub async fn execute(&self, id: &str, ctx: &AgentContext) -> Result<AgentResult> {
        let executor = self
            .get(id)
            .ok_or_else(|| crate::errors::AstrBotError::NotFound(format!("agent: {}", id)))?;
        executor.execute(ctx).await
    }

    /// Execute with multi-turn tool calling loop
    /// If the agent is a ToolCallingAgentExecutor, this is already handled internally.
    /// For other executors, falls back to single-turn execute.
    pub async fn execute_loop(&self, id: &str, ctx: &AgentContext) -> Result<AgentResult> {
        self.execute(id, ctx).await
    }

    /// Health check all executors
    pub async fn health_check_all(&self) -> Vec<(String, bool)> {
        let mut results = Vec::new();
        for (id, executor) in &self.executors {
            let healthy = executor.health_check().await.unwrap_or(false);
            results.push((id.clone(), healthy));
        }
        results
    }
}
