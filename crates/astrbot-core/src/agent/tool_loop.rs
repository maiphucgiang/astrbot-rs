//! Multi-turn tool calling loop for Agent execution
//!
//! Pattern: user input → LLM → tool_calls → execute tool → result回传 → 循环至完成或 max_iterations

use super::{AgentContext, AgentResult, ToolCall, ToolResult};
use crate::errors::Result;
use crate::provider::{ChatConfig, ChatMessage};
use serde_json::json;

/// Maximum iterations before auto-truncation
const DEFAULT_MAX_ITERATIONS: usize = 5;

/// Trait for executing tools in the loop
#[async_trait::async_trait]
pub trait ToolExecutor: Send + Sync {
    /// Execute a single tool call and return the result
    async fn execute(&self, call: &ToolCall, ctx: &AgentContext) -> Result<ToolResult>;

    /// List available tools for schema generation
    fn tool_schemas(&self) -> Vec<serde_json::Value>;
}

/// A tool executor backed by a closure — useful for quick tests and small integrations
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

/// Multi-turn tool-calling agent executor
///
/// Runs a loop: LLM call → parse tool_calls → execute tools → feed results back → repeat.
pub struct ToolCallingAgentExecutor {
    max_iterations: usize,
}

impl ToolCallingAgentExecutor {
    pub fn new() -> Self {
        Self {
            max_iterations: DEFAULT_MAX_ITERATIONS,
        }
    }

    pub fn with_max_iterations(mut self, n: usize) -> Self {
        self.max_iterations = n;
        self
    }

    /// Execute the tool calling loop with an external LLM caller and tool executor
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

        for _iteration in 0..self.max_iterations {
            let response = llm_caller.call(&messages, &tool_schemas).await?;

            // Check if the model wants to call tools
            let tool_calls = response
                .get("tool_calls")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();

            if tool_calls.is_empty() {
                // No tools requested — return final text response
                let content = response
                    .get("content")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                return Ok(AgentResult::Text { content });
            }

            // Model requested tool calls — execute them
            let assistant_msg = json!({
                "role": "assistant",
                "content": response.get("content").unwrap_or(&json!("")),
                "tool_calls": tool_calls,
            });
            messages.push(assistant_msg);

            for call_raw in tool_calls {
                let call = parse_tool_call(&call_raw)?;
                let result = executor.execute(&call, ctx).await?;

                let tool_msg = json!({
                    "role": "tool",
                    "tool_call_id": result.call_id,
                    "content": result.output.to_string(),
                });
                messages.push(tool_msg);
            }
        }

        // Max iterations exceeded — return last assistant message as fallback
        Ok(AgentResult::Text {
            content: "[Max iterations reached]".to_string(),
        })
    }

    /// Health check — always returns true for the loop framework itself
    pub fn health_check(&self) -> bool {
        true
    }
}

impl Default for ToolCallingAgentExecutor {
    fn default() -> Self {
        Self::new()
    }
}

/// Trait for calling an LLM with tool support
#[async_trait::async_trait]
pub trait LlmCaller: Send + Sync {
    async fn call(
        &self,
        messages: &[serde_json::Value],
        tools: &[serde_json::Value],
    ) -> Result<serde_json::Value>;
}

/// Convert a ChatMessage to a serde_json::Value for LLM API consumption
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

/// Parse a raw tool call from LLM response into our ToolCall struct
fn parse_tool_call(raw: &serde_json::Value) -> Result<ToolCall> {
    let id = raw
        .get("id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let function = raw.get("function").ok_or_else(|| {
        crate::errors::AstrBotError::Serialization("Missing 'function' in tool_call".to_string())
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

// ========== Tests ==========

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::ChatMessage;

    /// A mock LLM caller for testing
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

        let ctx = AgentContext::new(
            crate::platform::MessageSource::new("test", "test", None),
            "user1".to_string(),
            "session1".to_string(),
        );

        let executor = ToolCallingAgentExecutor::new();
        let result = executor.execute_loop(&tools, &llm, &ctx).await.unwrap();

        match result {
            AgentResult::Text { content } => assert_eq!(content, "No tools needed"),
            _ => panic!("Expected direct text response"),
        }
    }

    #[tokio::test]
    async fn test_fn_executor() {
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

        let llm = MockLlmCaller::new(vec![
            json!({
                "content": "Calling echo",
                "tool_calls": [
                    {
                        "id": "call_1",
                        "function": { "name": "echo", "arguments": "{\"msg\":\"hi\"}" }
                    }
                ]
            }),
            json!({ "content": "Done" }),
        ]);

        let mut ctx = AgentContext::new(
            crate::platform::MessageSource::new("test", "test", None),
            "user1".to_string(),
            "session1".to_string(),
        );
        ctx.messages = vec![ChatMessage::user("Say hi")];

        let executor = ToolCallingAgentExecutor::new();
        let result = executor.execute_loop(&tools, &llm, &ctx).await.unwrap();

        match result {
            AgentResult::Text { content } => assert_eq!(content, "Done"),
            _ => panic!("Expected text result"),
        }
    }

    #[tokio::test]
    async fn test_max_iterations_protection() {
        // LLM always returns tool_calls → should hit max_iterations
        let llm = MockLlmCaller::new(
            (0..10)
                .map(|i| {
                    json!({
                        "content": format!("Iteration {}", i),
                        "tool_calls": [
                            {
                                "id": format!("call_{}", i),
                                "function": { "name": "noop", "arguments": "{}" }
                            }
                        ]
                    })
                })
                .collect(),
        );

        let tools = FnToolExecutor::new(
            vec![json!({
                "type": "function",
                "function": { "name": "noop", "description": "No operation" }
            })],
            |call, _ctx| {
                Ok(ToolResult {
                    call_id: call.id.clone(),
                    success: true,
                    output: json!({}),
                    error: None,
                })
            },
        );

        let mut ctx = AgentContext::new(
            crate::platform::MessageSource::new("test", "test", None),
            "user1".to_string(),
            "session1".to_string(),
        );
        ctx.messages = vec![ChatMessage::user("Loop forever")];

        let executor = ToolCallingAgentExecutor::new().with_max_iterations(2);
        let result = executor.execute_loop(&tools, &llm, &ctx).await.unwrap();

        match result {
            AgentResult::Text { content } => {
                assert!(content.contains("Max iterations"));
            }
            _ => panic!("Expected max-iterations fallback"),
        }
    }
}
