//! DeerFlow Agent Runner — LangGraph 风格 Workflow 图编排执行引擎
//!
//! 核心概念：
//! - `WorkflowNode`：工作流节点（Start / LlmCall / ToolCall / Conditional / End）
//! - `WorkflowGraph`：节点 + 边（edges）的有向图
//! - `DeerFlowEngine`：图执行引擎，支持串行/条件分支执行
//!
//! 配置：
//! - `workflow_json`：工作流定义（JSON）
//! - `api_key` / `base_url` / `model`：LLM 调用参数
//! - `system_prompt`：可选系统提示
//!
//! 执行方式：
//! 1. 解析 workflow JSON → 节点图
//! 2. 从 `start` 节点开始执行
//! 3. 每个节点操作共享 `State`（HashMap）
//! 4. `llm_call` 节点调用 LLM，结果写入 state
//! 5. `tool_call` 节点调用工具（通过外部注册）
//! 6. `conditional` 节点根据 state 条件决定下一跳
//! 7. `end` 节点返回最终文本
//!
//! 当前版本：线性串行执行（所有节点按顺序执行），条件分支为基础 if/else。

use async_trait::async_trait;
use crate::errors::{AstrBotError, Result};
use crate::platform::MessageSource;
use crate::provider::ChatMessage;
use super::{AgentContext, AgentResult, AgentExecutor, AgentConfig, ToolResult};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use tracing::{info, warn};

// ---------------------------------------------------------------------------
// Workflow Node Definitions
// ---------------------------------------------------------------------------

/// A node in the DeerFlow workflow graph.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WorkflowNode {
    /// Entry point — initializes state.
    Start {
        id: String,
        #[serde(default)]
        next: Option<String>,
    },
    /// LLM call — sends messages to an LLM and stores the response.
    LlmCall {
        id: String,
        /// Optional prompt template (uses `{{user_input}}` and `{{state_key}}` placeholders)
        prompt_template: Option<String>,
        #[serde(default)]
        model: Option<String>,
        #[serde(default)]
        temperature: Option<f32>,
        #[serde(default)]
        max_tokens: Option<u32>,
        /// Key in state to write the response to (default: `"llm_output"`)
        #[serde(default = "default_output_key")]
        output_key: String,
        #[serde(default)]
        next: Option<String>,
    },
    /// Tool call — invokes one or more tools.
    ToolCall {
        id: String,
        /// Tool names to call (must be registered externally)
        tools: Vec<String>,
        /// Input key in state (default: `"llm_output"`)
        #[serde(default = "default_input_key")]
        input_key: String,
        /// Output key in state (default: `"tool_output"`)
        #[serde(default = "default_tool_output_key")]
        output_key: String,
        #[serde(default)]
        next: Option<String>,
    },
    /// Conditional branch — decides the next node based on state.
    Conditional {
        id: String,
        /// Condition expression: simple key existence or equality check
        condition: ConditionExpr,
        /// Next node if condition is true
        #[serde(default)]
        then_next: Option<String>,
        /// Next node if condition is false
        #[serde(default)]
        else_next: Option<String>,
    },
    /// Terminal node — returns the final result.
    End {
        id: String,
        /// Key in state to read the final output from (default: `"llm_output"`)
        #[serde(default = "default_output_key")]
        output_key: String,
    },
}

fn default_output_key() -> String { "llm_output".to_string() }
fn default_input_key() -> String { "llm_output".to_string() }
fn default_tool_output_key() -> String { "tool_output".to_string() }

/// Simple condition expression for conditional nodes.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub enum ConditionExpr {
    /// Check if a key exists in state and is truthy.
    #[serde(rename = "has")]
    Has { key: String },
    /// Check if a key equals a JSON value.
    #[serde(rename = "eq")]
    Eq { key: String, value: Value },
    /// Check if a key contains a substring.
    #[serde(rename = "contains")]
    Contains { key: String, substring: String },
}

impl ConditionExpr {
    pub fn evaluate(&self, state: &State) -> bool {
        match self {
            ConditionExpr::Has { key } => {
                state.get(key).map(|v| !is_falsy(v)).unwrap_or(false)
            }
            ConditionExpr::Eq { key, value } => {
                state.get(key).map(|v| v == value).unwrap_or(false)
            }
            ConditionExpr::Contains { key, substring } => {
                state.get(key)
                    .and_then(|v| v.as_str())
                    .map(|s| s.contains(substring))
                    .unwrap_or(false)
            }
        }
    }
}

fn is_falsy(v: &Value) -> bool {
    match v {
        Value::Null => true,
        Value::Bool(b) => !*b,
        Value::String(s) => s.is_empty(),
        Value::Array(a) => a.is_empty(),
        Value::Object(o) => o.is_empty(),
        _ => false,
    }
}

// ---------------------------------------------------------------------------
// Workflow Graph
// ---------------------------------------------------------------------------

/// A workflow definition parsed from JSON.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct WorkflowDefinition {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    pub nodes: Vec<WorkflowNode>,
    /// Global edges (fallback if node-level `next` is not set).
    #[serde(default)]
    pub edges: HashMap<String, String>,
}

/// Compiled workflow graph for execution.
pub struct WorkflowGraph {
    pub definition: WorkflowDefinition,
    /// Node lookup by ID.
    pub node_map: HashMap<String, WorkflowNode>,
}

impl WorkflowGraph {
    pub fn from_definition(def: WorkflowDefinition) -> Result<Self> {
        let mut node_map = HashMap::new();
        for node in &def.nodes {
            let id = node_id(node);
            if node_map.contains_key(&id) {
                return Err(AstrBotError::Config(format!("workflow.node.{}: Duplicate node id: {}", id, id)));
            }
            node_map.insert(id.clone(), node.clone());
        }
        // Validate start node exists
        if !node_map.contains_key("start") {
            return Err(AstrBotError::Config(format!("{}: {}", "workflow.start".to_string(), "Workflow must have a 'start' node".to_string(),)));
        }
        Ok(Self {
            definition: def,
            node_map,
        })
    }

    pub fn get_node(&self, id: &str) -> Option<&WorkflowNode> {
        self.node_map.get(id)
    }

    pub fn next_node_id(&self, current: &WorkflowNode, state: &State) -> Option<String> {
        match current {
            WorkflowNode::Start { next, .. } => next.clone(),
            WorkflowNode::LlmCall { next, .. } => next.clone(),
            WorkflowNode::ToolCall { next, .. } => next.clone(),
            WorkflowNode::Conditional { condition, then_next, else_next, .. } => {
                if condition.evaluate(state) {
                    then_next.clone()
                } else {
                    else_next.clone()
                }
            }
            WorkflowNode::End { .. } => None,
        }
        .or_else(|| self.definition.edges.get(&node_id(current)).cloned())
    }
}

fn node_id(node: &WorkflowNode) -> String {
    match node {
        WorkflowNode::Start { id, .. } => id.clone(),
        WorkflowNode::LlmCall { id, .. } => id.clone(),
        WorkflowNode::ToolCall { id, .. } => id.clone(),
        WorkflowNode::Conditional { id, .. } => id.clone(),
        WorkflowNode::End { id, .. } => id.clone(),
    }
}

// ---------------------------------------------------------------------------
// Execution State
// ---------------------------------------------------------------------------

/// Mutable state shared across workflow nodes.
pub type State = HashMap<String, Value>;

/// Execution context for a single workflow run.
pub struct WorkflowRunContext {
    pub state: State,
    pub user_input: String,
    pub messages: Vec<ChatMessage>,
}

// ---------------------------------------------------------------------------
// LLM Client (OpenAI-compatible)
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct LlmClient {
    api_key: String,
    base_url: String,
    default_model: String,
    http_client: reqwest::Client,
}

impl LlmClient {
    pub fn new(api_key: String, base_url: String, default_model: String) -> Self {
        Self {
            api_key,
            base_url,
            default_model,
            http_client: reqwest::Client::new(),
        }
    }

    fn auth_headers(&self) -> reqwest::header::HeaderMap {
        let mut h = reqwest::header::HeaderMap::new();
        h.insert(
            reqwest::header::AUTHORIZATION,
            reqwest::header::HeaderValue::from_str(&format!("Bearer {}", self.api_key)).unwrap(),
        );
        h.insert(
            reqwest::header::CONTENT_TYPE,
            reqwest::header::HeaderValue::from_static("application/json"),
        );
        h
    }

    /// Render a prompt template with placeholders.
    fn render_template(&self, template: &str, state: &State, user_input: &str) -> String {
        let mut result = template.to_string();
        result = result.replace("{{user_input}}", user_input);
        // Replace {{state.KEY}} with state value
        for (key, value) in state {
            let placeholder = format!("{{state.{}}}", key);
            let val_str = match value {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            result = result.replace(&placeholder, &val_str);
        }
        result
    }

    pub async fn chat(
        &self,
        messages: Vec<ChatMessage>,
        prompt_template: Option<&str>,
        state: &State,
        user_input: &str,
        model_override: Option<&str>,
        temperature: Option<f32>,
        max_tokens: Option<u32>,
    ) -> Result<String> {
        let mut msgs: Vec<serde_json::Value> = messages.iter().map(|m| {
            json!({
                "role": m.role,
                "content": m.content,
            })
        }).collect();

        if let Some(template) = prompt_template {
            let rendered = self.render_template(template, state, user_input);
            msgs.push(json!({
                "role": "user",
                "content": rendered,
            }));
        } else if !user_input.is_empty() {
            msgs.push(json!({
                "role": "user",
                "content": user_input,
            }));
        }

        let body = json!({
            "model": model_override.unwrap_or(&self.default_model),
            "messages": msgs,
            "temperature": temperature.unwrap_or(0.7),
            "max_tokens": max_tokens,
        });

        let url = format!("{}/v1/chat/completions", self.base_url.trim_end_matches('/'));
        let resp = self.http_client
            .post(&url)
            .headers(self.auth_headers())
            .json(&body)
            .send()
            .await
            .map_err(|e| AstrBotError::Network(format!("DeerFlow LLM call failed: {}", e)))?;

        let status = resp.status();
        let body_text = resp.text().await.unwrap_or_default();

        if !status.is_success() {
            return Err(AstrBotError::Provider {
                provider: "deerflow".to_string(),
                message: format!("DeerFlow LLM error {}: {}", status, body_text),
            });
        }

        let parsed: serde_json::Value = serde_json::from_str(&body_text)
            .map_err(|e| AstrBotError::Serialization(format!(
                "DeerFlow LLM parse error: {} — body: {}",
                e,
                &body_text[..body_text.len().min(500)]
            )))?;

        let content = parsed
            .get("choices")
            .and_then(|c| c.as_array())
            .and_then(|arr| arr.first())
            .and_then(|c| c.get("message"))
            .and_then(|m| m.get("content"))
            .and_then(|c| c.as_str())
            .unwrap_or("")
            .to_string();

        Ok(content)
    }
}

// ---------------------------------------------------------------------------
// DeerFlow Engine
// ---------------------------------------------------------------------------

/// The DeerFlow workflow execution engine.
pub struct DeerFlowEngine {
    graph: WorkflowGraph,
    llm_client: LlmClient,
    /// Registered tool callbacks: name -> fn(input: Value) -> Result<Value>
    tools: HashMap<String, Box<dyn Fn(Value) -> Result<Value> + Send + Sync>>,
}

impl DeerFlowEngine {
    pub fn new(graph: WorkflowGraph, llm_client: LlmClient) -> Self {
        Self {
            graph,
            llm_client,
            tools: HashMap::new(),
        }
    }

    pub fn register_tool(
        &mut self,
        name: impl Into<String>,
        callback: impl Fn(Value) -> Result<Value> + Send + Sync + 'static,
    ) {
        self.tools.insert(name.into(), Box::new(callback));
    }

    /// Execute the workflow from the `start` node.
    pub async fn run(
        &self,
        ctx: &WorkflowRunContext,
    ) -> Result<AgentResult> {
        let mut current_id = "start".to_string();
        let mut state = ctx.state.clone();

        // Seed user_input into state
        state.insert("user_input".to_string(), Value::String(ctx.user_input.clone()));

        let max_steps = 100usize;
        let mut steps = 0usize;

        while steps < max_steps {
            steps += 1;
            let node = self.graph.get_node(&current_id)
                .ok_or_else(|| AstrBotError::NotFound(format!(
                    "workflow node: {}", current_id
                )))?;

            match node {
                WorkflowNode::Start { .. } => {
                    // No-op, just transitions
                }
                WorkflowNode::LlmCall {
                    prompt_template,
                    model,
                    temperature,
                    max_tokens,
                    output_key,
                    ..
                } => {
                    let response = self.llm_client.chat(
                        ctx.messages.clone(),
                        prompt_template.as_deref(),
                        &state,
                        &ctx.user_input,
                        model.as_deref(),
                        *temperature,
                        *max_tokens,
                    ).await?;
                    state.insert(output_key.clone(), Value::String(response));
                }
                WorkflowNode::ToolCall {
                    tools: tool_names,
                    input_key,
                    output_key,
                    ..
                } => {
                    let input = state.get(input_key.as_str()).cloned().unwrap_or(Value::Null);
                    let mut results = Vec::new();
                    for tool_name in tool_names {
                        if let Some(callback) = self.tools.get(tool_name) {
                            match callback(input.clone()) {
                                Ok(result) => results.push(json!({
                                    "tool": tool_name,
                                    "success": true,
                                    "output": result,
                                })),
                                Err(e) => results.push(json!({
                                    "tool": tool_name,
                                    "success": false,
                                    "error": format!("{}", e),
                                })),
                            }
                        } else {
                            results.push(json!({
                                "tool": tool_name,
                                "success": false,
                                "error": format!("Tool '{}' not registered", tool_name),
                            }));
                        }
                    }
                    state.insert(output_key.clone(), Value::Array(results));
                }
                WorkflowNode::Conditional { condition, .. } => {
                    // Transition handled below
                    let _ = condition;
                }
                WorkflowNode::End { output_key, .. } => {
                    let output = state.get(output_key.as_str())
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    return Ok(AgentResult::Text { content: output });
                }
            }

            // Determine next node
            if let Some(next_id) = self.graph.next_node_id(node, &state) {
                current_id = next_id;
            } else {
                // No explicit next — if current is End we already returned;
                // otherwise treat as end with current output.
                let fallback_key = match node {
                    WorkflowNode::LlmCall { output_key, .. } => Some(output_key.clone()),
                    WorkflowNode::ToolCall { output_key, .. } => Some(output_key.clone()),
                    _ => None,
                };
                let output = fallback_key
                    .and_then(|k| state.get(&k))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                return Ok(AgentResult::Text { content: output });
            }
        }

        Err(AstrBotError::Internal("DeerFlow workflow exceeded max steps (100)".to_string()))
    }
}

// ---------------------------------------------------------------------------
// DeerFlow Agent Runner
// ---------------------------------------------------------------------------

pub struct DeerFlowAgentRunner {
    engine: DeerFlowEngine,
    system_prompt: Option<String>,
}

impl DeerFlowAgentRunner {
    pub fn new(config: &AgentConfig) -> Result<Self> {
        let api_key = config
            .config
            .get("api_key")
            .and_then(|v| v.as_str())
            .ok_or_else(|| AstrBotError::Config(format!("{}: {}", "api_key".to_string(), "Missing DeerFlow API key".to_string(),)))?
            .to_string();

        let base_url = config
            .config
            .get("base_url")
            .and_then(|v| v.as_str())
            .unwrap_or("https://api.openai.com")
            .to_string();

        let model = config
            .config
            .get("model")
            .and_then(|v| v.as_str())
            .unwrap_or("gpt-3.5-turbo")
            .to_string();

        let workflow_json = config
            .config
            .get("workflow_json")
            .ok_or_else(|| AstrBotError::Config(format!("{}: {}", "workflow_json".to_string(), "Missing DeerFlow workflow JSON".to_string(),)))?;

        let workflow_def: WorkflowDefinition = serde_json::from_value(workflow_json.clone())
            .map_err(|e| AstrBotError::Serialization(format!(
                "DeerFlow workflow parse error: {}", e
            )))?;

        let graph = WorkflowGraph::from_definition(workflow_def)?;
        let llm_client = LlmClient::new(api_key, base_url, model);
        let engine = DeerFlowEngine::new(graph, llm_client);

        Ok(Self {
            engine,
            system_prompt: config.system_prompt.clone(),
        })
    }

    pub fn engine_mut(&mut self) -> &mut DeerFlowEngine {
        &mut self.engine
    }
}

#[async_trait]
impl AgentExecutor for DeerFlowAgentRunner {
    fn name(&self) -> &str {
        &self.engine.graph.definition.name
    }

    fn executor_type(&self) -> &str {
        "deerflow"
    }

    async fn initialize(&mut self, config: serde_json::Value) -> Result<()> {
        if let Some(url) = config.get("base_url").and_then(|v| v.as_str()) {
            self.engine.llm_client.base_url = url.to_string();
        }
        if let Some(model) = config.get("model").and_then(|v| v.as_str()) {
            self.engine.llm_client.default_model = model.to_string();
        }
        if let Some(prompt) = config.get("system_prompt").and_then(|v| v.as_str()) {
            self.system_prompt = Some(prompt.to_string());
        }
        info!("[DeerFlow] Runner initialized — workflow={}", self.name());
        Ok(())
    }

    async fn execute(&self, ctx: &AgentContext) -> Result<AgentResult> {
        let user_input = ctx.messages.last()
            .map(|m| m.content.clone())
            .unwrap_or_default();

        let mut messages = ctx.messages.clone();
        if let Some(ref prompt) = self.system_prompt {
            messages.insert(0, ChatMessage::system(prompt.clone()));
        }

        let mut state = State::new();
        if !ctx.extras.is_empty() {
            for (k, v) in &ctx.extras {
                state.insert(k.clone(), v.clone());
            }
        }

        let wf_ctx = WorkflowRunContext {
            state,
            user_input,
            messages,
        };

        self.engine.run(&wf_ctx).await
    }

    async fn execute_with_tools(
        &self,
        ctx: &AgentContext,
        tool_results: Vec<ToolResult>,
    ) -> Result<AgentResult> {
        // DeerFlow handles tools internally via ToolCall nodes.
        // For external tool results, feed them into state and re-run.
        let mut state = State::new();
        for tr in tool_results {
            state.insert(
                format!("tool_result_{}", tr.call_id),
                json!({
                    "success": tr.success,
                    "output": tr.output,
                    "error": tr.error,
                }),
            );
        }
        let user_input = ctx.messages.last()
            .map(|m| m.content.clone())
            .unwrap_or_default();

        let mut messages = ctx.messages.clone();
        if let Some(ref prompt) = self.system_prompt {
            messages.insert(0, ChatMessage::system(prompt.clone()));
        }

        let wf_ctx = WorkflowRunContext {
            state,
            user_input,
            messages,
        };

        self.engine.run(&wf_ctx).await
    }

    async fn health_check(&self) -> Result<bool> {
        // Lightweight: check if LLM endpoint is reachable.
        let url = format!(
            "{}/v1/models",
            self.engine.llm_client.base_url.trim_end_matches('/')
        );
        let resp = self.engine.llm_client.http_client
            .get(&url)
            .header(reqwest::header::AUTHORIZATION, format!("Bearer {}", self.engine.llm_client.api_key))
            .send()
            .await;
        match resp {
            Ok(r) => Ok(r.status().is_success() || r.status().as_u16() == 401),
            Err(_) => Ok(false),
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::AgentConfig;
    use crate::platform::MessageSource;
    use crate::provider::ChatMessage;

    fn make_test_config(workflow: Value) -> AgentConfig {
        AgentConfig {
            id: "test-deerflow".to_string(),
            name: "Test DeerFlow".to_string(),
            executor_type: "deerflow".to_string(),
            enabled: true,
            config: json!({
                "api_key": "test-key",
                "base_url": "https://api.openai.com",
                "model": "gpt-3.5-turbo",
                "workflow_json": workflow,
            }),
            system_prompt: Some("You are a helpful assistant.".to_string()),
            enable_tools: false,
            max_iterations: 5,
        }
    }

    fn make_test_context() -> AgentContext {
        AgentContext {
            messages: vec![
                ChatMessage::user("Hello"),
            ],
            source: MessageSource {
                platform: crate::platform::PlatformType::Custom,
                session_id: "test-chat".to_string(),
                message_id: "msg-1".to_string(),
                user_id: "test-user".to_string(),
            },
            user_id: "test-user".to_string(),
            session_id: "test-session".to_string(),
            extras: HashMap::new(),
        }
    }

    fn linear_workflow() -> Value {
        json!({
            "name": "linear-chat",
            "description": "A simple linear workflow: start -> llm -> end",
            "nodes": [
                { "type": "start", "id": "start", "next": "llm1" },
                { "type": "llm_call", "id": "llm1", "prompt_template": "User said: {{user_input}}", "output_key": "llm_output", "next": "end" },
                { "type": "end", "id": "end", "output_key": "llm_output" }
            ]
        })
    }

    fn conditional_workflow() -> Value {
        json!({
            "name": "conditional-chat",
            "description": "Branch based on user input containing a keyword",
            "nodes": [
                { "type": "start", "id": "start", "next": "llm1" },
                { "type": "llm_call", "id": "llm1", "prompt_template": "Respond to: {{user_input}}", "output_key": "llm_output", "next": "check" },
                { "type": "conditional", "id": "check", "condition": { "has": { "key": "llm_output" } }, "then_next": "end", "else_next": "llm1" },
                { "type": "end", "id": "end", "output_key": "llm_output" }
            ]
        })
    }

    fn tool_workflow() -> Value {
        json!({
            "name": "tool-workflow",
            "description": "Call a tool then end",
            "nodes": [
                { "type": "start", "id": "start", "next": "tool1" },
                { "type": "tool_call", "id": "tool1", "tools": ["echo"], "input_key": "user_input", "output_key": "tool_output", "next": "end" },
                { "type": "end", "id": "end", "output_key": "tool_output" }
            ]
        })
    }

    // ------------------------------------------------------------------
    // Workflow parsing tests
    // ------------------------------------------------------------------

    #[test]
    fn test_workflow_parse_linear() {
        let wf: WorkflowDefinition = serde_json::from_value(linear_workflow()).unwrap();
        assert_eq!(wf.name, "linear-chat");
        assert_eq!(wf.nodes.len(), 3);
    }

    #[test]
    fn test_workflow_parse_conditional() {
        let wf: WorkflowDefinition = serde_json::from_value(conditional_workflow()).unwrap();
        assert_eq!(wf.name, "conditional-chat");
        let graph = WorkflowGraph::from_definition(wf).unwrap();
        assert!(graph.get_node("check").is_some());
    }

    #[test]
    fn test_workflow_missing_start() {
        let bad = json!({
            "name": "bad",
            "nodes": [
                { "type": "llm_call", "id": "llm1", "output_key": "out" }
            ]
        });
        let wf: WorkflowDefinition = serde_json::from_value(bad).unwrap();
        let err = WorkflowGraph::from_definition(wf).unwrap_err();
        let msg = format!("{}", err);
        assert!(msg.contains("start"));
    }

    #[test]
    fn test_workflow_duplicate_node() {
        let bad = json!({
            "name": "bad",
            "nodes": [
                { "type": "start", "id": "start" },
                { "type": "llm_call", "id": "start", "output_key": "out" }
            ]
        });
        let wf: WorkflowDefinition = serde_json::from_value(bad).unwrap();
        let err = WorkflowGraph::from_definition(wf).unwrap_err();
        let msg = format!("{}", err);
        assert!(msg.contains("Duplicate"));
    }

    // ------------------------------------------------------------------
    // Condition evaluation tests
    // ------------------------------------------------------------------

    #[test]
    fn test_condition_has() {
        let mut state = State::new();
        let cond = ConditionExpr::Has { key: "foo".to_string() };
        assert!(!cond.evaluate(&state));
        state.insert("foo".to_string(), Value::String("bar".to_string()));
        assert!(cond.evaluate(&state));
    }

    #[test]
    fn test_condition_eq() {
        let mut state = State::new();
        let cond = ConditionExpr::Eq { key: "num".to_string(), value: Value::Number(42.into()) };
        assert!(!cond.evaluate(&state));
        state.insert("num".to_string(), Value::Number(42.into()));
        assert!(cond.evaluate(&state));
    }

    #[test]
    fn test_condition_contains() {
        let mut state = State::new();
        let cond = ConditionExpr::Contains { key: "text".to_string(), substring: "world".to_string() };
        assert!(!cond.evaluate(&state));
        state.insert("text".to_string(), Value::String("hello world".to_string()));
        assert!(cond.evaluate(&state));
    }

    #[test]
    fn test_condition_falsy() {
        let mut state = State::new();
        let cond = ConditionExpr::Has { key: "empty".to_string() };
        state.insert("empty".to_string(), Value::String("".to_string()));
        assert!(!cond.evaluate(&state));
        state.insert("empty".to_string(), Value::Array(vec![]));
        assert!(!cond.evaluate(&state));
    }

    #[test]
    fn test_graph_next_node_linear() {
        let wf: WorkflowDefinition = serde_json::from_value(linear_workflow()).unwrap();
        let graph = WorkflowGraph::from_definition(wf).unwrap();
        let start = graph.get_node("start").unwrap();
        assert_eq!(graph.next_node_id(start, &State::new()), Some("llm1".to_string()));

        let llm = graph.get_node("llm1").unwrap();
        assert_eq!(graph.next_node_id(llm, &State::new()), Some("end".to_string()));

        let end = graph.get_node("end").unwrap();
        assert_eq!(graph.next_node_id(end, &State::new()), None);
    }

    #[test]
    fn test_graph_next_node_conditional() {
        let wf: WorkflowDefinition = serde_json::from_value(conditional_workflow()).unwrap();
        let graph = WorkflowGraph::from_definition(wf).unwrap();
        let cond = graph.get_node("check").unwrap();

        let mut state_with = State::new();
        state_with.insert("llm_output".to_string(), Value::String("yes".to_string()));
        assert_eq!(graph.next_node_id(cond, &state_with), Some("end".to_string()));

        let mut state_without = State::new();
        assert_eq!(graph.next_node_id(cond, &state_without), Some("llm1".to_string()));
    }

    // ------------------------------------------------------------------
    // LLM client tests (mock server)
    // ------------------------------------------------------------------

    async fn run_mock_llm_server(port: u16, response_body: String) {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", port))
            .await
            .unwrap();
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut buf = vec![0u8; 4096];
        let n = stream.read(&mut buf).await.unwrap_or(0);
        let _req = String::from_utf8_lossy(&buf[..n]);
        let http_response = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/json\r\n\r\n{}",
            response_body.len(),
            response_body
        );
        stream.write_all(http_response.as_bytes()).await.unwrap();
    }

    #[tokio::test]
    async fn test_llm_client_chat() {
        let port = 29999u16;
        let body = r#"{"choices":[{"message":{"role":"assistant","content":"Mocked LLM reply"}}]}"#;
        let server = tokio::spawn(run_mock_llm_server(port, body.to_string()));
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        let client = LlmClient::new(
            "fake-key".to_string(),
            format!("http://127.0.0.1:{}", port),
            "gpt-test".to_string(),
        );
        let state = State::new();
        let result = client.chat(
            vec![ChatMessage::user("hi")],
            Some("Say hello to {{user_input}}"),
            &state,
            "Alice",
            None,
            None,
            None,
        ).await.unwrap();

        assert_eq!(result, "Mocked LLM reply");
        let _ = server.await;
    }

    #[tokio::test]
    async fn test_llm_client_no_template() {
        let port = 29998u16;
        let body = r#"{"choices":[{"message":{"role":"assistant","content":"Direct reply"}}]}"#;
        let server = tokio::spawn(run_mock_llm_server(port, body.to_string()));
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        let client = LlmClient::new(
            "fake-key".to_string(),
            format!("http://127.0.0.1:{}", port),
            "gpt-test".to_string(),
        );
        let state = State::new();
        let result = client.chat(
            vec![ChatMessage::user("hi")],
            None,
            &state,
            "direct input",
            None,
            None,
            None,
        ).await.unwrap();

        assert_eq!(result, "Direct reply");
        let _ = server.await;
    }

    // ------------------------------------------------------------------
    // Engine integration tests (mock LLM)
    // ------------------------------------------------------------------

    #[tokio::test]
    async fn test_engine_linear_workflow() {
        let port = 29997u16;
        let body = r#"{"choices":[{"message":{"role":"assistant","content":"Engine output"}}]}"#;
        let server = tokio::spawn(run_mock_llm_server(port, body.to_string()));
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        let mut config = make_test_config(linear_workflow());
        config.config["base_url"] = json!(format!("http://127.0.0.1:{}", port));

        let runner = DeerFlowAgentRunner::new(&config).unwrap();
        let ctx = make_test_context();
        let result = runner.execute(&ctx).await.unwrap();

        match result {
            AgentResult::Text { content } => {
                assert_eq!(content, "Engine output");
            }
            _ => panic!("Expected Text result"),
        }

        let _ = server.await;
    }

    #[tokio::test]
    async fn test_engine_conditional_workflow_then_branch() {
        let port = 29996u16;
        let body = r#"{"choices":[{"message":{"role":"assistant","content":"conditional yes"}}]}"#;
        let server = tokio::spawn(run_mock_llm_server(port, body.to_string()));
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        let mut config = make_test_config(conditional_workflow());
        config.config["base_url"] = json!(format!("http://127.0.0.1:{}", port));

        let runner = DeerFlowAgentRunner::new(&config).unwrap();
        let ctx = make_test_context();
        let result = runner.execute(&ctx).await.unwrap();

        match result {
            AgentResult::Text { content } => {
                assert_eq!(content, "conditional yes");
            }
            _ => panic!("Expected Text result"),
        }

        let _ = server.await;
    }

    #[tokio::test]
    async fn test_engine_tool_workflow() {
        let port = 29995u16;
        // Tool workflow doesn't call LLM, so no mock server needed for LLM.
        let mut config = make_test_config(tool_workflow());
        config.config["base_url"] = json!(format!("http://127.0.0.1:{}", port));

        let mut runner = DeerFlowAgentRunner::new(&config).unwrap();
        runner.engine_mut().register_tool("echo", |input| {
            let s = input.as_str().unwrap_or("");
            Ok(Value::String(format!("echo: {}", s)))
        });

        let mut ctx = make_test_context();
        ctx.extras.insert("user_input".to_string(), Value::String("hello".to_string()));

        let result = runner.execute(&ctx).await.unwrap();

        match result {
            AgentResult::Text { content } => {
                assert!(content.contains("echo: hello"));
            }
            _ => panic!("Expected Text result with tool output"),
        }
    }

    #[tokio::test]
    async fn test_engine_max_steps_guard() {
        // Create a workflow with a cycle to test max_steps guard
        let cycle_workflow = json!({
            "name": "cycle",
            "nodes": [
                { "type": "start", "id": "start", "next": "a" },
                { "type": "llm_call", "id": "a", "output_key": "out", "next": "b" },
                { "type": "conditional", "id": "b", "condition": { "has": { "key": "never" } }, "then_next": "end", "else_next": "a" },
                { "type": "end", "id": "end", "output_key": "out" }
            ]
        });

        let port = 29994u16;
        let body = r#"{"choices":[{"message":{"role":"assistant","content":"loop"}}]}"#;
        let server = tokio::spawn(run_mock_llm_server(port, body.to_string()));
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        let mut config = make_test_config(cycle_workflow);
        config.config["base_url"] = json!(format!("http://127.0.0.1:{}", port));

        let runner = DeerFlowAgentRunner::new(&config).unwrap();
        let ctx = make_test_context();
        let result = runner.execute(&ctx).await;

        assert!(result.is_err());
        let err_msg = format!("{}", result.unwrap_err());
        assert!(err_msg.contains("max steps"));

        let _ = server.await;
    }

    #[tokio::test]
    async fn test_health_check() {
        let port = 29993u16;
        let response = "HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\n\r\n".to_string();
        let server = tokio::spawn(async move {
            let listener = tokio::net::TcpListener::bind(("127.0.0.1", port))
                .await
                .unwrap();
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut buf = vec![0u8; 4096];
            let n = stream.read(&mut buf).await.unwrap_or(0);
            let _req = String::from_utf8_lossy(&buf[..n]);
            stream.write_all(response.as_bytes()).await.unwrap();
        });
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        let mut config = make_test_config(linear_workflow());
        config.config["base_url"] = json!(format!("http://127.0.0.1:{}", port));

        let runner = DeerFlowAgentRunner::new(&config).unwrap();
        let healthy = runner.health_check().await.unwrap();
        assert!(healthy); // 401 means service is reachable

        let _ = server.await;
    }

    #[test]
    fn test_runner_creation() {
        let config = make_test_config(linear_workflow());
        let runner = DeerFlowAgentRunner::new(&config).unwrap();
        assert_eq!(runner.name(), "linear-chat");
        assert_eq!(runner.executor_type(), "deerflow");
    }

    #[test]
    fn test_runner_missing_workflow() {
        let bad_config = AgentConfig {
            id: "bad".to_string(),
            name: "Bad".to_string(),
            executor_type: "deerflow".to_string(),
            enabled: true,
            config: json!({
                "api_key": "key",
            }),
            system_prompt: None,
            enable_tools: false,
            max_iterations: 5,
        };
        let err = DeerFlowAgentRunner::new(&bad_config).unwrap_err();
        let msg = format!("{}", err);
        assert!(msg.contains("workflow_json"));
    }

    #[test]
    fn test_runner_missing_api_key() {
        let mut config = make_test_config(linear_workflow());
        config.config = json!({
            "workflow_json": linear_workflow(),
        });
        let err = DeerFlowAgentRunner::new(&config).unwrap_err();
        let msg = format!("{}", err);
        assert!(msg.contains("api_key"));
    }

    #[test]
    fn test_llm_template_rendering() {
        let client = LlmClient::new(
            "k".to_string(),
            "https://example.com".to_string(),
            "m".to_string(),
        );
        let mut state = State::new();
        state.insert("name".to_string(), Value::String("Alice".to_string()));
        state.insert("age".to_string(), Value::Number(30.into()));

        let rendered = client.render_template(
            "Hello {{state.name}}, you are {{state.age}}. User said: {{user_input}}",
            &state,
            "hi",
        );
        assert_eq!(rendered, "Hello Alice, you are 30. User said: hi");
    }
}
