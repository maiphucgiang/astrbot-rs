use crate::errors::{AstrBotError, Result};
use crate::message::MessageChain;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{error, info, warn};

// ---------------------------------------------------------------------------
// Tool definitions
// ---------------------------------------------------------------------------

/// Parameter definition for a tool
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolParameter {
    pub name: String,
    pub description: String,
    pub param_type: String,  // "string", "number", "boolean", "array", "object"
    pub required: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enum_values: Option<Vec<String>>,
}

/// Tool definition (metadata)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub parameters: Vec<ToolParameter>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub returns: Option<String>,
    #[serde(default)]
    pub requires_confirmation: bool,
}

impl ToolDefinition {
    /// Convert to OpenAI-compatible function schema
    pub fn to_openai_schema(&self) -> Value {
        let mut properties = serde_json::Map::new();
        let mut required = Vec::new();

        for param in &self.parameters {
            let mut prop = serde_json::Map::new();
            prop.insert("type".to_string(), Value::String(param.param_type.clone()));
            prop.insert("description".to_string(), Value::String(param.description.clone()));
            if let Some(vals) = &param.enum_values {
                prop.insert("enum".to_string(), Value::Array(vals.iter().map(|v| Value::String(v.clone())).collect()));
            }
            properties.insert(param.name.clone(), Value::Object(prop));

            if param.required {
                required.push(Value::String(param.name.clone()));
            }
        }

        serde_json::json!({
            "type": "function",
            "function": {
                "name": self.name,
                "description": self.description,
                "parameters": {
                    "type": "object",
                    "properties": properties,
                    "required": required,
                }
            }
        })
    }
}

/// A parsed tool call from LLM response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: Value,
}

/// Result of executing a tool
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ToolResult {
    /// Tool executed successfully
    Success { output: Value },
    /// Tool execution failed
    Error { message: String },
    /// Tool requires user confirmation before execution
    NeedsConfirmation { tool_call: ToolCall, reason: String },
}

// ---------------------------------------------------------------------------
// Tool trait and registry
// ---------------------------------------------------------------------------

/// A tool that can be called by the agent / LLM
#[async_trait]
pub trait Tool: Send + Sync {
    /// Get tool definition
    fn definition(&self) -> &ToolDefinition;
    /// Execute the tool with parsed arguments
    async fn execute(&self, arguments: &Value) -> Result<ToolResult>;
    /// Check if execution needs confirmation
    fn needs_confirmation(&self, _arguments: &Value) -> bool {
        self.definition().requires_confirmation
    }
}

/// Tool registry — manages all available tools
pub struct ToolRegistry {
    tools: RwLock<HashMap<String, Box<dyn Tool>>>,
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self {
            tools: RwLock::new(HashMap::new()),
        }
    }

    /// Register a tool
    pub async fn register(&self, tool: Box<dyn Tool>) {
        let name = tool.definition().name.clone();
        let mut tools = self.tools.write().await;
        tools.insert(name, tool);
    }

    /// Unregister a tool
    pub async fn unregister(&self, name: &str) {
        let mut tools = self.tools.write().await;
        tools.remove(name);
    }

    /// Get tool definition
    pub async fn get_definition(&self, name: &str) -> Option<ToolDefinition> {
        let tools = self.tools.read().await;
        tools.get(name).map(|t| t.definition().clone())
    }

    /// List all tool definitions
    pub async fn list_definitions(&self) -> Vec<ToolDefinition> {
        let tools = self.tools.read().await;
        tools.values().map(|t| t.definition().clone()).collect()
    }

    /// Execute a tool by name
    pub async fn execute(
        &self,
        name: &str,
        arguments: &Value,
    ) -> Result<ToolResult> {
        let tools = self.tools.read().await;
        let tool = tools.get(name)
            .ok_or_else(|| AstrBotError::NotFound(format!("tool: {}", name)))?;

        if tool.needs_confirmation(arguments) {
            return Ok(ToolResult::NeedsConfirmation {
                tool_call: ToolCall {
                    id: uuid::Uuid::new_v4().to_string(),
                    name: name.to_string(),
                    arguments: arguments.clone(),
                },
                reason: "This tool requires confirmation".to_string(),
            });
        }

        tool.execute(arguments).await
    }

    /// Check if a tool exists
    pub async fn has_tool(&self, name: &str) -> bool {
        let tools = self.tools.read().await;
        tools.contains_key(name)
    }

    /// Get all tools as OpenAI function schemas
    pub async fn to_openai_schemas(&self) -> Vec<Value> {
        let defs = self.list_definitions().await;
        defs.into_iter().map(|d| d.to_openai_schema()).collect()
    }
}

// ---------------------------------------------------------------------------
// Built-in tools
// ---------------------------------------------------------------------------

/// Echo tool — returns the input back (for testing)
pub struct EchoTool {
    definition: ToolDefinition,
}

impl EchoTool {
    pub fn new() -> Self {
        Self {
            definition: ToolDefinition {
                name: "echo".to_string(),
                description: "Echo back the input text".to_string(),
                parameters: vec![
                    ToolParameter {
                        name: "text".to_string(),
                        description: "Text to echo".to_string(),
                        param_type: "string".to_string(),
                        required: true,
                        default: None,
                        enum_values: None,
                    },
                ],
                returns: Some("string".to_string()),
                requires_confirmation: false,
            },
        }
    }
}

#[async_trait]
impl Tool for EchoTool {
    fn definition(&self) -> &ToolDefinition {
        &self.definition
    }

    async fn execute(&self, arguments: &Value) -> Result<ToolResult> {
        let text = arguments.get("text")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        Ok(ToolResult::Success {
            output: Value::String(text),
        })
    }
}

/// Current time tool
pub struct CurrentTimeTool {
    definition: ToolDefinition,
}

impl CurrentTimeTool {
    pub fn new() -> Self {
        Self {
            definition: ToolDefinition {
                name: "current_time".to_string(),
                description: "Get the current date and time".to_string(),
                parameters: vec![],
                returns: Some("string".to_string()),
                requires_confirmation: false,
            },
        }
    }
}

#[async_trait]
impl Tool for CurrentTimeTool {
    fn definition(&self) -> &ToolDefinition {
        &self.definition
    }

    async fn execute(&self, _arguments: &Value) -> Result<ToolResult> {
        let now = chrono::Local::now();
        Ok(ToolResult::Success {
            output: Value::String(now.to_rfc3339()),
        })
    }
}

// ---------------------------------------------------------------------------
// Function call parser (OpenAI format)
// ---------------------------------------------------------------------------

/// Parse function calls from OpenAI-style response
pub fn parse_openai_tool_calls(content: &str) -> Vec<ToolCall> {
    let mut calls = Vec::new();

    // Try to find JSON arrays or objects in the content
    if let Ok(Value::Array(arr)) = serde_json::from_str(content) {
        for item in arr {
            if let Some(obj) = item.as_object() {
                if let (Some(id), Some(name), Some(args)) = (
                    obj.get("id").and_then(|v| v.as_str()),
                    obj.get("name").or_else(|| obj.get("function").and_then(|f| f.get("name"))).and_then(|v| v.as_str()),
                    obj.get("arguments").or_else(|| obj.get("function").and_then(|f| f.get("arguments"))),
                ) {
                    calls.push(ToolCall {
                        id: id.to_string(),
                        name: name.to_string(),
                        arguments: args.clone(),
                    });
                }
            }
        }
    }

    calls
}

/// Build a tool result message for LLM context
pub fn tool_result_message(tool_call_id: &str, result: &ToolResult) -> Value {
    match result {
        ToolResult::Success { output } => {
            serde_json::json!({
                "tool_call_id": tool_call_id,
                "role": "tool",
                "content": output.to_string(),
            })
        }
        ToolResult::Error { message } => {
            serde_json::json!({
                "tool_call_id": tool_call_id,
                "role": "tool",
                "content": format!("Error: {}", message),
            })
        }
        ToolResult::NeedsConfirmation { .. } => {
            serde_json::json!({
                "tool_call_id": tool_call_id,
                "role": "tool",
                "content": "Awaiting user confirmation...",
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_tool_registry() {
        let registry = ToolRegistry::new();
        let echo = Box::new(EchoTool::new());
        registry.register(echo).await;

        assert!(registry.has_tool("echo").await);
        let defs = registry.list_definitions().await;
        assert_eq!(defs.len(), 1);
        assert_eq!(defs[0].name, "echo");

        let result = registry.execute("echo", &serde_json::json!({"text": "hello"})).await.unwrap();
        match result {
            ToolResult::Success { output } => {
                assert_eq!(output, Value::String("hello".to_string()));
            }
            _ => panic!("Expected success"),
        }
    }

    #[tokio::test]
    async fn test_current_time_tool() {
        let tool = CurrentTimeTool::new();
        let result = tool.execute(&serde_json::json!({})).await.unwrap();
        assert!(matches!(result, ToolResult::Success { .. }));
    }

    #[test]
    fn test_openai_schema_conversion() {
        let def = ToolDefinition {
            name: "test".to_string(),
            description: "A test tool".to_string(),
            parameters: vec![
                ToolParameter {
                    name: "arg1".to_string(),
                    description: "First arg".to_string(),
                    param_type: "string".to_string(),
                    required: true,
                    default: None,
                    enum_values: Some(vec!["a".to_string(), "b".to_string()]),
                },
            ],
            returns: None,
            requires_confirmation: false,
        };

        let schema = def.to_openai_schema();
        assert!(schema.get("type").is_some());
        assert!(schema.get("function").is_some());
    }

    #[test]
    fn test_parse_openai_tool_calls() {
        let json = r#"[{"id":"call_1","name":"echo","arguments":{"text":"hi"}}]"#;
        let calls = parse_openai_tool_calls(json);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "echo");
    }
}
