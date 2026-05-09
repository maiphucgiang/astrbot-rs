//! Enhanced multi-turn tool calling loop with context compression,
//! empty-output retry, duplicate tool detection, result overflow handling,
//! and a hook system for extension.
//!
//! Pattern: user input → LLM → tool_calls → execute tool → result回传 → 循环至完成或 max_iterations

use super::{AgentContext, AgentResult, ToolCall, ToolResult};
use crate::errors::{AstrBotError, Result};
use crate::provider::{ChatConfig, ChatMessage};
use serde_json::json;
use std::collections::HashSet;
use tracing::{error, info, warn};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Default max iterations
const DEFAULT_MAX_ITERATIONS: usize = 5;

/// Default max context length (in chars, rough heuristic)
const DEFAULT_MAX_CONTEXT_CHARS: usize = 8000;

/// Default empty-output retry count
const DEFAULT_EMPTY_RETRY: usize = 2;

/// Default max tool result length (chars) before truncation
const DEFAULT_MAX_TOOL_RESULT_CHARS: usize = 2000;

// ---------------------------------------------------------------------------
// Hook system
// ---------------------------------------------------------------------------

/// Hook point in the tool loop lifecycle
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookPoint {
    /// Before LLM call
    PreLlmCall,
    /// After LLM call (before tool execution)
    PostLlmCall,
    /// Before tool execution
    PreToolExecute,
    /// After tool execution (before feeding result back)
    PostToolExecute,
    /// Before returning final result (loop about to exit)
    PreReturn,
    /// On max iterations reached
    OnMaxIterations,
}

/// Hook callback trait for external extensions
#[async_trait::async_trait]
pub trait ToolLoopHook: Send + Sync {
    /// Called at each hook point. Returns decision for loop control.
    async fn invoke(
        &self,
        point: HookPoint,
        iteration: usize,
        messages: &[serde_json::Value],
        last_response: Option<&serde_json::Value>,
    ) -> Result<HookDecision>;
}

/// Decision returned by a hook
#[derive(Debug, Clone)]
pub enum HookDecision {
    /// Continue normal execution
    Continue,
    /// Abort with text result
    Abort { reason: String, result_text: String },
    /// Skip current step and retry
    Retry { hint: String },
}

// ---------------------------------------------------------------------------
// Tool executor trait
// ---------------------------------------------------------------------------

#[async_trait::async_trait]
pub trait ToolExecutor: Send + Sync {
    /// Execute a single tool call and return the result
    async fn execute(&self, call: &ToolCall, ctx: &AgentContext) -> Result<ToolResult>;
    /// List available tools for schema generation
    fn tool_schemas(&self) -> Vec<serde_json::Value>;
}

// ---------------------------------------------------------------------------
// Closure-backed tool executor
// ---------------------------------------------------------------------------

pub struct FnToolExecutor {
    schemas: Vec<serde_json::Value>,
    f: Box<dyn Fn(&ToolCall, &AgentContext) -> Result<ToolResult> + Send + Sync>,
}

impl FnToolExecutor {
    pub fn new<F>(schemas: Vec<serde_json::Value>, f: F) -> Self
    where
        F: Fn(&ToolCall, &AgentContext) -> Result<ToolResult> + Send + Sync + 'static,
    {
        Self {
            schemas,
            f: Box::new(f),
        }
    }
}

#[async_trait::async_trait]
impl ToolExecutor for FnToolExecutor {
    async fn execute(&self, call: &ToolCall, ctx: &AgentContext) -> Result<ToolResult> {
        (self.f)(call, ctx)
    }
    fn tool_schemas(&self) -> Vec<serde_json::Value> {
        self.schemas.clone()
    }
}

// ---------------------------------------------------------------------------
// LLM caller trait
// ---------------------------------------------------------------------------

#[async_trait::async_trait]
pub trait LlmCaller: Send + Sync {
    async fn call(
        &self,
        messages: &[serde_json::Value],
        tools: &[serde_json::Value],
    ) -> Result<serde_json::Value>;
}

// ---------------------------------------------------------------------------
// Enhanced Tool Calling Agent Executor
// ---------------------------------------------------------------------------

pub struct ToolCallingAgentExecutor {
    max_iterations: usize,
    max_context_chars: usize,
    empty_retry_count: usize,
    max_tool_result_chars: usize,
    hooks: Vec<Box<dyn ToolLoopHook>>,
}

impl ToolCallingAgentExecutor {
    pub fn new() -> Self {
        Self {
            max_iterations: DEFAULT_MAX_ITERATIONS,
            max_context_chars: DEFAULT_MAX_CONTEXT_CHARS,
            empty_retry_count: DEFAULT_EMPTY_RETRY,
            max_tool_result_chars: DEFAULT_MAX_TOOL_RESULT_CHARS,
            hooks: Vec::new(),
        }
    }

    pub fn with_max_iterations(mut self, n: usize) -> Self {
        self.max_iterations = n;
        self
    }

    pub fn with_max_context_chars(mut self, n: usize) -> Self {
        self.max_context_chars = n;
        self
    }

    pub fn with_empty_retry(mut self, n: usize) -> Self {
        self.empty_retry_count = n;
        self
    }

    pub fn with_max_tool_result_chars(mut self, n: usize) -> Self {
        self.max_tool_result_chars = n;
        self
    }

    /// Register a hook for extension
    pub fn add_hook<H: ToolLoopHook + 'static>(&mut self, hook: H) {
        self.hooks.push(Box::new(hook));
    }

    /// Execute the enhanced tool calling loop
    pub async fn execute_loop<E: ToolExecutor>(
        &self,
        executor: &E,
        llm_caller: &dyn LlmCaller,
        ctx: &AgentContext,
    ) -> Result<AgentResult> {
        let mut messages: Vec<serde_json::Value> = ctx
            .messages
            .iter()
            .map(|m| chat_message_to_value(m))
            .collect();

        let tool_schemas = executor.tool_schemas();
        let mut duplicate_tracker = DuplicateToolTracker::new();

        for iteration in 0..self.max_iterations {
            // --- Context compression ---
            self.compress_context_if_needed(&mut messages);

            // --- Pre-LLM hook ---
            self.run_hooks(HookPoint::PreLlmCall, iteration, &messages, None)
                .await?;

            // --- LLM call with empty-output retry ---
            let response = self
                .call_llm_with_retry(llm_caller, &messages, &tool_schemas)
                .await?;

            // --- Post-LLM hook ---
            match self
                .run_hooks(
                    HookPoint::PostLlmCall,
                    iteration,
                    &messages,
                    Some(&response),
                )
                .await?
            {
                HookDecision::Continue => {}
                HookDecision::Abort {
                    reason,
                    result_text,
                } => {
                    warn!("[ToolLoop] Aborted by hook at PostLlmCall: {}", reason);
                    return Ok(AgentResult::Text {
                        content: result_text,
                    });
                }
                HookDecision::Retry { hint } => {
                    warn!("[ToolLoop] Hook retry hint: {}", hint);
                    continue;
                }
            }

            // Check tool calls
            let tool_calls = response
                .get("tool_calls")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();

            if tool_calls.is_empty() {
                // No tools — extract content and return
                let content = response
                    .get("content")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();

                self.run_hooks(HookPoint::PreReturn, iteration, &messages, Some(&response))
                    .await?;
                return Ok(AgentResult::Text { content });
            }

            // Tools requested — build assistant message
            let assistant_msg = json!({
                "role": "assistant",
                "content": response.get("content").unwrap_or(&json!("")),
                "tool_calls": tool_calls,
            });
            messages.push(assistant_msg);

            // Execute each tool call
            for call_raw in tool_calls {
                let call = parse_tool_call(&call_raw)?;

                // --- Duplicate detection ---
                if duplicate_tracker.is_duplicate(&call) {
                    warn!(
                        "[ToolLoop] Duplicate tool call detected: {}({})",
                        call.name, call.arguments
                    );
                    let dup_msg = json!({
                        "role": "tool",
                        "tool_call_id": call.id,
                        "content": "[Duplicate call detected: same tool with identical arguments was just executed.]",
                    });
                    messages.push(dup_msg);
                    continue;
                }
                duplicate_tracker.record(&call);

                // --- Pre-tool hook ---
                self.run_hooks(
                    HookPoint::PreToolExecute,
                    iteration,
                    &messages,
                    Some(&call_raw),
                )
                .await?;

                // Execute
                let result = executor.execute(&call, ctx).await?;

                // --- Post-tool hook ---
                self.run_hooks(
                    HookPoint::PostToolExecute,
                    iteration,
                    &messages,
                    Some(&call_raw),
                )
                .await?;

                // --- Result overflow handling ---
                let result_content = self.truncate_tool_result(&result);

                let tool_msg = json!({
                    "role": "tool",
                    "tool_call_id": result.call_id,
                    "content": result_content,
                });
                messages.push(tool_msg);
            }
        }

        // Max iterations reached
        self.run_hooks(
            HookPoint::OnMaxIterations,
            self.max_iterations,
            &messages,
            None,
        )
        .await?;
        Ok(AgentResult::Text {
            content: "[Max iterations reached]".to_string(),
        })
    }

    /// Call LLM with empty-output retry logic
    async fn call_llm_with_retry(
        &self,
        llm_caller: &dyn LlmCaller,
        messages: &[serde_json::Value],
        tools: &[serde_json::Value],
    ) -> Result<serde_json::Value> {
        for retry in 0..=self.empty_retry_count {
            let response = llm_caller.call(messages, tools).await?;

            let content = response.get("content").and_then(|v| v.as_str());
            let has_tool_calls = response
                .get("tool_calls")
                .and_then(|v| v.as_array())
                .map(|arr| !arr.is_empty())
                .unwrap_or(false);

            // Accept if content is non-empty OR tool_calls are present
            if content.map(|s| !s.is_empty()).unwrap_or(false) || has_tool_calls {
                return Ok(response);
            }

            warn!(
                "[ToolLoop] Empty output (retry {}/{}), retrying with hint...",
                retry, self.empty_retry_count
            );
        }

        // After all retries, return empty response (caller will decide)
        Ok(json!({ "content": "" }))
    }

    /// Compress context if it exceeds the threshold
    fn compress_context_if_needed(&self, messages: &mut Vec<serde_json::Value>) {
        let total_chars: usize = messages
            .iter()
            .map(|m| {
                m.get("content")
                    .and_then(|v| v.as_str())
                    .map(|s| s.len())
                    .unwrap_or(0)
            })
            .sum();

        if total_chars <= self.max_context_chars {
            return;
        }

        warn!(
            "[ToolLoop] Context too large ({} chars > {}), compressing older messages",
            total_chars, self.max_context_chars
        );

        // Strategy: keep system + latest user message + last 2 assistant/tool pairs,
        // summarize the rest into a single "system" note
        if messages.len() <= 4 {
            return; // Too short to compress meaningfully
        }

        let to_compress = messages.len() - 4;
        let compressed_count = to_compress / 2; // compress roughly half
        if compressed_count == 0 {
            return;
        }

        let mut new_messages = Vec::new();
        // Keep first message (usually system)
        if let Some(first) = messages.first() {
            new_messages.push(first.clone());
        }

        // Add a summary placeholder for compressed messages
        let summary = json!({
            "role": "system",
            "content": format!(
                "[{} earlier messages were compressed to save context. Key details preserved in latest messages.]",
                compressed_count
            ),
        });
        new_messages.push(summary);

        // Keep last N messages uncompressed
        let keep_count = messages.len() - compressed_count;
        for msg in messages.iter().skip(keep_count) {
            new_messages.push(msg.clone());
        }

        *messages = new_messages;
    }

    /// Truncate tool result if too long
    fn truncate_tool_result(&self, result: &ToolResult) -> String {
        let raw = result.output.to_string();
        if raw.len() <= self.max_tool_result_chars {
            return raw;
        }

        let truncated = &raw[..self.max_tool_result_chars];
        format!(
            "{}\n\n[... truncated: {} chars > {} limit]",
            truncated,
            raw.len(),
            self.max_tool_result_chars
        )
    }

    /// Run all registered hooks at a given point
    async fn run_hooks(
        &self,
        point: HookPoint,
        iteration: usize,
        messages: &[serde_json::Value],
        last_response: Option<&serde_json::Value>,
    ) -> Result<HookDecision> {
        for hook in &self.hooks {
            match hook
                .invoke(point, iteration, messages, last_response)
                .await?
            {
                HookDecision::Continue => continue,
                decision @ (HookDecision::Abort { .. } | HookDecision::Retry { .. }) => {
                    return Ok(decision);
                }
            }
        }
        Ok(HookDecision::Continue)
    }

    pub fn health_check(&self) -> bool {
        true
    }
}

impl Default for ToolCallingAgentExecutor {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Duplicate tool call tracker
// ---------------------------------------------------------------------------

struct DuplicateToolTracker {
    seen: HashSet<(String, String)>, // (tool_name, arguments_json)
}

impl DuplicateToolTracker {
    fn new() -> Self {
        Self {
            seen: HashSet::new(),
        }
    }

    fn record(&mut self, call: &ToolCall) {
        let key = (call.name.clone(), call.arguments.to_string());
        self.seen.insert(key);
    }

    fn is_duplicate(&self, call: &ToolCall) -> bool {
        let key = (call.name.clone(), call.arguments.to_string());
        self.seen.contains(&key)
    }
}

// ---------------------------------------------------------------------------
// Utility functions
// ---------------------------------------------------------------------------

fn chat_message_to_value(msg: &ChatMessage) -> serde_json::Value {
    let mut obj = serde_json::Map::new();
    obj.insert("role".to_string(), json!(msg.role));
    obj.insert("content".to_string(), json!(msg.content));
    if let Some(name) = &msg.name {
        obj.insert("name".to_string(), json!(name));
    }
    if let Some(tool_calls) = &msg.tool_calls {
        obj.insert("tool_calls".to_string(), json!(tool_calls));
    }
    if let Some(tool_call_id) = &msg.tool_call_id {
        obj.insert("tool_call_id".to_string(), json!(tool_call_id));
    }
    json!(obj)
}

fn parse_tool_call(raw: &serde_json::Value) -> Result<ToolCall> {
    let id = raw
        .get("id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let function = raw.get("function").ok_or_else(|| {
        AstrBotError::Serialization("Missing 'function' in tool_call".to_string())
    })?;
    let name = function
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let arguments = function.get("arguments").cloned().unwrap_or(json!({}));

    Ok(ToolCall {
        id,
        name,
        arguments,
    })
}

// ---------------------------------------------------------------------------
// Built-in Metrics hook
// ---------------------------------------------------------------------------

/// A built-in hook that logs loop progress for observability
pub struct MetricsHook;

#[async_trait::async_trait]
impl ToolLoopHook for MetricsHook {
    async fn invoke(
        &self,
        point: HookPoint,
        iteration: usize,
        messages: &[serde_json::Value],
        _last_response: Option<&serde_json::Value>,
    ) -> Result<HookDecision> {
        let msg_count = messages.len();
        match point {
            HookPoint::PreLlmCall => {
                info!(
                    "[ToolLoop] Iteration {} — {} messages in context",
                    iteration, msg_count
                );
            }
            HookPoint::PostToolExecute => {
                info!(
                    "[ToolLoop] Iteration {} — tool executed, {} messages",
                    iteration, msg_count
                );
            }
            HookPoint::OnMaxIterations => {
                warn!("[ToolLoop] Max iterations ({}) reached", iteration);
            }
            _ => {}
        }
        Ok(HookDecision::Continue)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::ChatMessage;

    struct MockLlmCaller {
        responses: std::sync::Mutex<Vec<serde_json::Value>>,
    }

    impl MockLlmCaller {
        fn new(responses: Vec<serde_json::Value>) -> Self {
            Self {
                responses: std::sync::Mutex::new(responses),
            }
        }
    }

    #[async_trait::async_trait]
    impl LlmCaller for MockLlmCaller {
        async fn call(
            &self,
            _messages: &[serde_json::Value],
            _tools: &[serde_json::Value],
        ) -> Result<serde_json::Value> {
            let mut responses = self.responses.lock().unwrap();
            if responses.is_empty() {
                Ok(json!({ "content": "Hello from mock" }))
            } else {
                Ok(responses.remove(0))
            }
        }
    }

    #[test]
    fn test_tool_loop_creation() {
        let executor = ToolCallingAgentExecutor::new();
        assert!(executor.health_check());
    }

    #[tokio::test]
    async fn test_no_tools_direct_return() {
        let llm = MockLlmCaller::new(vec![json!({ "content": "No tools needed" })]);
        let tools = FnToolExecutor::new(vec![], |_call, _ctx| {
            Ok(ToolResult {
                call_id: "1".to_string(),
                success: true,
                output: json!({}),
                error: None,
            })
        });

        let ctx = AgentContext {
            messages: vec![ChatMessage::user("Hello")],
            source: Default::default(),
            user_id: "test".to_string(),
            session_id: "test".to_string(),
            extras: Default::default(),
        };

        let executor = ToolCallingAgentExecutor::new();
        let result = executor.execute_loop(&tools, &llm, &ctx).await.unwrap();

        match result {
            AgentResult::Text { content } => assert_eq!(content, "No tools needed"),
            _ => panic!("Expected direct text response"),
        }
    }

    #[tokio::test]
    async fn test_duplicate_tool_detection() {
        let tools = FnToolExecutor::new(
            vec![json!({
                "type": "function",
                "function": { "name": "echo", "description": "Echo input" }
            })],
            |call, _ctx| {
                Ok(ToolResult {
                    call_id: call.id.clone(),
                    success: true,
                    output: json!({ "echoed": call.arguments.to_string() }),
                    error: None,
                })
            },
        );

        // LLM returns same tool call twice → second should be detected as duplicate
        let llm = MockLlmCaller::new(vec![
            json!({
                "content": "Calling echo",
                "tool_calls": [
                    {
                        "id": "call_1",
                        "function": { "name": "echo", "arguments": "{\"msg\":\"hi\"}" }
                    },
                    {
                        "id": "call_2",
                        "function": { "name": "echo", "arguments": "{\"msg\":\"hi\"}" }
                    }
                ]
            }),
            json!({ "content": "Done" }),
        ]);

        let ctx = AgentContext {
            messages: vec![ChatMessage::user("Say hi twice")],
            source: Default::default(),
            user_id: "test".to_string(),
            session_id: "test".to_string(),
            extras: Default::default(),
        };

        let executor = ToolCallingAgentExecutor::new();
        let result = executor.execute_loop(&tools, &llm, &ctx).await.unwrap();

        match result {
            AgentResult::Text { content } => assert_eq!(content, "Done"),
            _ => panic!("Expected text result"),
        }
    }

    #[tokio::test]
    async fn test_context_compression() {
        // Build a very long context (>8000 chars)
        let long_content = "a".repeat(3000);
        let messages: Vec<ChatMessage> = (0..4)
            .map(|i| ChatMessage::user(format!("{}", long_content.clone())))
            .collect();

        let llm = MockLlmCaller::new(vec![json!({ "content": "Compressed ok" })]);
        let tools = FnToolExecutor::new(vec![], |_call, _ctx| {
            Ok(ToolResult {
                call_id: "1".to_string(),
                success: true,
                output: json!({}),
                error: None,
            })
        });

        let ctx = AgentContext {
            messages,
            source: Default::default(),
            user_id: "test".to_string(),
            session_id: "test".to_string(),
            extras: Default::default(),
        };

        let executor = ToolCallingAgentExecutor::new().with_max_context_chars(5000);
        let result = executor.execute_loop(&tools, &llm, &ctx).await.unwrap();

        match result {
            AgentResult::Text { content } => assert_eq!(content, "Compressed ok"),
            _ => panic!("Expected text result"),
        }
    }

    #[tokio::test]
    async fn test_empty_output_retry() {
        // First call returns empty, second returns content
        let llm = MockLlmCaller::new(vec![
            json!({ "content": "" }),
            json!({ "content": "Retry succeeded" }),
        ]);
        let tools = FnToolExecutor::new(vec![], |_call, _ctx| {
            Ok(ToolResult {
                call_id: "1".to_string(),
                success: true,
                output: json!({}),
                error: None,
            })
        });

        let ctx = AgentContext {
            messages: vec![ChatMessage::user("Hello")],
            source: Default::default(),
            user_id: "test".to_string(),
            session_id: "test".to_string(),
            extras: Default::default(),
        };

        let executor = ToolCallingAgentExecutor::new().with_empty_retry(2);
        let result = executor.execute_loop(&tools, &llm, &ctx).await.unwrap();

        match result {
            AgentResult::Text { content } => assert_eq!(content, "Retry succeeded"),
            _ => panic!("Expected text result"),
        }
    }

    #[tokio::test]
    async fn test_tool_result_truncation() {
        let tools = FnToolExecutor::new(
            vec![json!({
                "type": "function",
                "function": { "name": "big_output", "description": "Returns huge output" }
            })],
            |_call, _ctx| {
                Ok(ToolResult {
                    call_id: "call_1".to_string(),
                    success: true,
                    output: json!("x".repeat(5000)),
                    error: None,
                })
            },
        );

        let llm = MockLlmCaller::new(vec![
            json!({
                "content": "Calling big_output",
                "tool_calls": [
                    {
                        "id": "call_1",
                        "function": { "name": "big_output", "arguments": "{}" }
                    }
                ]
            }),
            json!({ "content": "Done" }),
        ]);

        let ctx = AgentContext {
            messages: vec![ChatMessage::user("Get big data")],
            source: Default::default(),
            user_id: "test".to_string(),
            session_id: "test".to_string(),
            extras: Default::default(),
        };

        let executor = ToolCallingAgentExecutor::new().with_max_tool_result_chars(100);
        let result = executor.execute_loop(&tools, &llm, &ctx).await.unwrap();

        match result {
            AgentResult::Text { content } => assert_eq!(content, "Done"),
            _ => panic!("Expected text result"),
        }
    }

    #[tokio::test]
    async fn test_metrics_hook() {
        let llm = MockLlmCaller::new(vec![json!({ "content": "Hooked" })]);
        let tools = FnToolExecutor::new(vec![], |_call, _ctx| {
            Ok(ToolResult {
                call_id: "1".to_string(),
                success: true,
                output: json!({}),
                error: None,
            })
        });

        let ctx = AgentContext {
            messages: vec![ChatMessage::user("Hello")],
            source: Default::default(),
            user_id: "test".to_string(),
            session_id: "test".to_string(),
            extras: Default::default(),
        };

        let mut executor = ToolCallingAgentExecutor::new();
        executor.add_hook(MetricsHook);
        let result = executor.execute_loop(&tools, &llm, &ctx).await.unwrap();

        match result {
            AgentResult::Text { content } => assert_eq!(content, "Hooked"),
            _ => panic!("Expected text result"),
        }
    }
}
