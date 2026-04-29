use crate::errors::{AstrBotError, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::{mpsc, oneshot, RwLock};
use tracing::{error, info, warn};

// ---------------------------------------------------------------------------
// MCP Protocol types
// https://modelcontextprotocol.io
// ---------------------------------------------------------------------------

/// MCP JSON-RPC request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpRequest {
    pub jsonrpc: String,
    pub id: Option<u64>,
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

/// MCP JSON-RPC response
#[derive(Debug, Clone, Deserialize)]
pub struct McpResponse {
    pub jsonrpc: String,
    pub id: Option<u64>,
    #[serde(flatten)]
    pub result_or_error: McpResultOrError,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum McpResultOrError {
    Result { result: Value },
    Error { error: McpErrorDetail },
}

#[derive(Debug, Clone, Deserialize)]
pub struct McpErrorDetail {
    pub code: i32,
    pub message: String,
    #[serde(default)]
    pub data: Option<Value>,
}

/// MCP tool definition
#[derive(Debug, Clone, Deserialize)]
pub struct McpTool {
    pub name: String,
    pub description: Option<String>,
    #[serde(default)]
    pub input_schema: Value,
}

/// MCP resource definition
#[derive(Debug, Clone, Deserialize)]
pub struct McpResource {
    pub uri: String,
    pub name: String,
    #[serde(default)]
    pub mime_type: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
}

/// MCP prompt definition
#[derive(Debug, Clone, Deserialize)]
pub struct McpPrompt {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub arguments: Option<Vec<McpPromptArgument>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct McpPromptArgument {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub required: Option<bool>,
}

/// Result of list_tools
#[derive(Debug, Clone, Deserialize)]
pub struct ListToolsResult {
    pub tools: Vec<McpTool>,
}

/// Result of list_resources
#[derive(Debug, Clone, Deserialize)]
pub struct ListResourcesResult {
    pub resources: Vec<McpResource>,
}

/// Result of list_prompts
#[derive(Debug, Clone, Deserialize)]
pub struct ListPromptsResult {
    pub prompts: Vec<McpPrompt>,
}

// ---------------------------------------------------------------------------
// MCP Transport trait
// ---------------------------------------------------------------------------

#[async_trait::async_trait]
pub trait McpTransport: Send + Sync {
    /// Send a request and wait for response
    async fn request(&mut self, req: McpRequest) -> Result<McpResponse>;
    /// Send a notification (no response expected)
    async fn notify(&mut self, req: McpRequest) -> Result<()>;
    /// Close the transport
    async fn close(&mut self) -> Result<()>;
}

// ---------------------------------------------------------------------------
// Stdio transport — spawn subprocess, communicate via stdin/stdout
// ---------------------------------------------------------------------------

/// Shared state between the transport and the background reader task
struct StdioShared {
    pending: RwLock<HashMap<u64, oneshot::Sender<Result<McpResponse>>>>,
    notification_tx: mpsc::UnboundedSender<McpResponse>,
}

pub struct StdioTransport {
    child: Child,
    stdin: ChildStdin,
    shared: Arc<StdioShared>,
    next_id: u64,
}

impl StdioTransport {
    pub async fn new(command: &str, args: &[String]) -> Result<Self> {
        let mut child = Command::new(command)
            .args(args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::inherit())
            .spawn()
            .map_err(|e| AstrBotError::Internal(format!("Failed to spawn MCP server: {}", e)))?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| AstrBotError::Internal("Failed to get child stdin".to_string()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| AstrBotError::Internal("Failed to get child stdout".to_string()))?;

        let reader = BufReader::new(stdout);
        let (notification_tx, _notification_rx) = mpsc::unbounded_channel();

        let shared = Arc::new(StdioShared {
            pending: RwLock::new(HashMap::new()),
            notification_tx: notification_tx.clone(),
        });

        // Spawn background reader task
        let shared_clone = Arc::clone(&shared);
        tokio::spawn(async move {
            Self::reader_loop(reader, shared_clone).await;
        });

        Ok(Self {
            child,
            stdin,
            shared,
            next_id: 1,
        })
    }

    /// Background task: read lines from stdout, parse responses, route to pending requests
    async fn reader_loop(mut reader: BufReader<ChildStdout>, shared: Arc<StdioShared>) {
        let mut line = String::new();
        loop {
            line.clear();
            match reader.read_line(&mut line).await {
                Ok(0) => {
                    info!("MCP stdio reader: EOF reached, closing");
                    break;
                }
                Ok(_) => {
                    let trimmed = line.trim();
                    if trimmed.is_empty() {
                        continue;
                    }
                    match serde_json::from_str::<McpResponse>(trimmed) {
                        Ok(resp) => {
                            let id = resp.id;
                            if let Some(id) = id {
                                let sender = {
                                    let mut pending = shared.pending.write().await;
                                    pending.remove(&id)
                                };
                                if let Some(tx) = sender {
                                    let _ = tx.send(Ok(resp));
                                } else {
                                    // No pending request — could be a notification or late response
                                    let _ = shared.notification_tx.send(resp);
                                }
                            } else {
                                // Notification (no id)
                                let _ = shared.notification_tx.send(resp);
                            }
                        }
                        Err(e) => {
                            warn!(
                                "MCP stdio reader: failed to parse line: {} | line: {}",
                                e, trimmed
                            );
                        }
                    }
                }
                Err(e) => {
                    error!("MCP stdio reader: read error: {}", e);
                    break;
                }
            }
        }
    }
}

#[async_trait::async_trait]
impl McpTransport for StdioTransport {
    async fn request(&mut self, req: McpRequest) -> Result<McpResponse> {
        let id = req.id.unwrap_or_else(|| {
            let id = self.next_id;
            self.next_id += 1;
            id
        });

        // Set up oneshot channel before sending request
        let (tx, rx) = oneshot::channel();
        {
            let mut pending = self.shared.pending.write().await;
            pending.insert(id, tx);
        }

        let json = serde_json::to_string(&req)
            .map_err(|e| AstrBotError::Serialization(format!("MCP serialize: {}", e)))?;

        self.stdin
            .write_all(format!("{}\n", json).as_bytes())
            .await
            .map_err(|e| AstrBotError::Network(format!("MCP stdio write: {}", e)))?;

        self.stdin
            .flush()
            .await
            .map_err(|e| AstrBotError::Network(format!("MCP stdio flush: {}", e)))?;

        // Wait for response with timeout
        match tokio::time::timeout(std::time::Duration::from_secs(30), rx).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(AstrBotError::Network(
                "MCP response channel closed".to_string(),
            )),
            Err(_) => {
                // Timeout — clean up pending entry
                let mut pending = self.shared.pending.write().await;
                pending.remove(&id);
                Err(AstrBotError::Network("MCP request timed out".to_string()))
            }
        }
    }

    async fn notify(&mut self, req: McpRequest) -> Result<()> {
        let json = serde_json::to_string(&req)
            .map_err(|e| AstrBotError::Serialization(format!("MCP serialize: {}", e)))?;

        self.stdin
            .write_all(format!("{}\n", json).as_bytes())
            .await
            .map_err(|e| AstrBotError::Network(format!("MCP stdio write: {}", e)))?;

        self.stdin
            .flush()
            .await
            .map_err(|e| AstrBotError::Network(format!("MCP stdio flush: {}", e)))?;

        Ok(())
    }

    async fn close(&mut self) -> Result<()> {
        let _ = self.child.kill().await;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// SSE transport — HTTP Server-Sent Events
// ---------------------------------------------------------------------------

pub struct SseTransport {
    base_url: String,
    client: reqwest::Client,
    next_id: u64,
}

impl SseTransport {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            client: reqwest::Client::new(),
            next_id: 1,
        }
    }
}

#[async_trait::async_trait]
impl McpTransport for SseTransport {
    async fn request(&mut self, req: McpRequest) -> Result<McpResponse> {
        let url = format!("{}/rpc", self.base_url);

        let resp = self
            .client
            .post(&url)
            .json(&req)
            .send()
            .await
            .map_err(|e| AstrBotError::Network(format!("MCP SSE request: {}", e)))?;

        let mcp_resp: McpResponse = resp
            .json()
            .await
            .map_err(|e| AstrBotError::Serialization(format!("MCP SSE parse: {}", e)))?;

        Ok(mcp_resp)
    }

    async fn notify(&mut self, req: McpRequest) -> Result<()> {
        let url = format!("{}/rpc", self.base_url);

        let _ = self
            .client
            .post(&url)
            .json(&req)
            .send()
            .await
            .map_err(|e| AstrBotError::Network(format!("MCP SSE notify: {}", e)))?;

        Ok(())
    }

    async fn close(&mut self) -> Result<()> {
        // SSE connections are stateless — nothing to close
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// MCP Client
// ---------------------------------------------------------------------------

pub struct McpClient {
    transport: Box<dyn McpTransport>,
    next_id: u64,
}

impl McpClient {
    pub fn new(transport: Box<dyn McpTransport>) -> Self {
        Self {
            transport,
            next_id: 1,
        }
    }

    fn next_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    /// Initialize connection (MCP handshake)
    pub async fn initialize(&mut self) -> Result<Value> {
        let req = McpRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(self.next_id()),
            method: "initialize".to_string(),
            params: Some(serde_json::json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": { "name": "AstrBot", "version": "0.1.0" }
            })),
        };

        let resp = self.transport.request(req).await?;
        match resp.result_or_error {
            McpResultOrError::Result { result } => Ok(result),
            McpResultOrError::Error { error } => Err(AstrBotError::Internal(format!(
                "MCP initialize error {}: {}",
                error.code, error.message
            ))),
        }
    }

    /// List available tools
    pub async fn list_tools(&mut self) -> Result<Vec<McpTool>> {
        let req = McpRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(self.next_id()),
            method: "tools/list".to_string(),
            params: None,
        };

        let resp = self.transport.request(req).await?;
        match resp.result_or_error {
            McpResultOrError::Result { result } => {
                let parsed: ListToolsResult = serde_json::from_value(result).map_err(|e| {
                    AstrBotError::Serialization(format!("MCP list_tools parse: {}", e))
                })?;
                Ok(parsed.tools)
            }
            McpResultOrError::Error { error } => Err(AstrBotError::Internal(format!(
                "MCP list_tools error {}: {}",
                error.code, error.message
            ))),
        }
    }

    /// Call a tool
    pub async fn call_tool(&mut self, name: &str, arguments: Value) -> Result<Value> {
        let req = McpRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(self.next_id()),
            method: "tools/call".to_string(),
            params: Some(serde_json::json!({
                "name": name,
                "arguments": arguments,
            })),
        };

        let resp = self.transport.request(req).await?;
        match resp.result_or_error {
            McpResultOrError::Result { result } => Ok(result),
            McpResultOrError::Error { error } => Err(AstrBotError::Internal(format!(
                "MCP call_tool error {}: {}",
                error.code, error.message
            ))),
        }
    }

    /// List available resources
    pub async fn list_resources(&mut self) -> Result<Vec<McpResource>> {
        let req = McpRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(self.next_id()),
            method: "resources/list".to_string(),
            params: None,
        };

        let resp = self.transport.request(req).await?;
        match resp.result_or_error {
            McpResultOrError::Result { result } => {
                let parsed: ListResourcesResult = serde_json::from_value(result).map_err(|e| {
                    AstrBotError::Serialization(format!("MCP list_resources parse: {}", e))
                })?;
                Ok(parsed.resources)
            }
            McpResultOrError::Error { error } => Err(AstrBotError::Internal(format!(
                "MCP list_resources error {}: {}",
                error.code, error.message
            ))),
        }
    }

    /// List available prompts
    pub async fn list_prompts(&mut self) -> Result<Vec<McpPrompt>> {
        let req = McpRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(self.next_id()),
            method: "prompts/list".to_string(),
            params: None,
        };

        let resp = self.transport.request(req).await?;
        match resp.result_or_error {
            McpResultOrError::Result { result } => {
                let parsed: ListPromptsResult = serde_json::from_value(result).map_err(|e| {
                    AstrBotError::Serialization(format!("MCP list_prompts parse: {}", e))
                })?;
                Ok(parsed.prompts)
            }
            McpResultOrError::Error { error } => Err(AstrBotError::Internal(format!(
                "MCP list_prompts error {}: {}",
                error.code, error.message
            ))),
        }
    }

    /// Close the connection
    pub async fn close(&mut self) -> Result<()> {
        self.transport.close().await
    }
}

// ---------------------------------------------------------------------------
// MCP Server registry — manage multiple MCP servers
// ---------------------------------------------------------------------------

/// Arc-wrapped MCP client for registry usage (allows shared mutable access)
pub type SharedMcpClient = Arc<RwLock<McpClient>>;

pub struct McpServerRegistry {
    servers: RwLock<HashMap<String, SharedMcpClient>>,
}

impl Default for McpServerRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl McpServerRegistry {
    pub fn new() -> Self {
        Self {
            servers: RwLock::new(HashMap::new()),
        }
    }

    /// Register a new MCP client
    pub async fn register(&self, name: impl Into<String>, client: McpClient) {
        let mut servers = self.servers.write().await;
        servers.insert(name.into(), Arc::new(RwLock::new(client)));
    }

    /// Remove a server
    pub async fn unregister(&self, name: &str) -> Result<()> {
        let mut servers = self.servers.write().await;
        if let Some(client) = servers.remove(name) {
            let mut guard = client.write().await;
            guard.close().await?;
        }
        Ok(())
    }

    /// Get a shared client reference
    pub async fn get(&self, name: &str) -> Option<SharedMcpClient> {
        let servers = self.servers.read().await;
        servers.get(name).cloned()
    }

    /// List all server names
    pub async fn list_names(&self) -> Vec<String> {
        let servers = self.servers.read().await;
        servers.keys().cloned().collect()
    }

    /// Collect tools from all servers
    pub async fn collect_all_tools(&self) -> Result<Vec<(String, McpTool)>> {
        let servers = self.servers.read().await;
        let mut all_tools = Vec::new();

        for (name, client) in servers.iter() {
            let mut guard = client.write().await;
            match guard.list_tools().await {
                Ok(tools) => {
                    for tool in tools {
                        all_tools.push((name.clone(), tool));
                    }
                }
                Err(e) => {
                    warn!("MCP server '{}' list_tools failed: {}", name, e);
                }
            }
        }

        Ok(all_tools)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mcp_request_serialize() {
        let req = McpRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(1),
            method: "initialize".to_string(),
            params: Some(serde_json::json!({"test": true})),
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("\"method\":\"initialize\""));
    }

    #[test]
    fn test_mcp_response_parse_result() {
        let json = r#"{"jsonrpc":"2.0","id":1,"result":{"tools":[]}}"#;
        let resp: McpResponse = serde_json::from_str(json).unwrap();
        match resp.result_or_error {
            McpResultOrError::Result { result } => {
                assert!(result.get("tools").is_some());
            }
            _ => panic!("Expected result"),
        }
    }

    #[test]
    fn test_mcp_response_parse_error() {
        let json =
            r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32600,"message":"Invalid request"}}"#;
        let resp: McpResponse = serde_json::from_str(json).unwrap();
        match resp.result_or_error {
            McpResultOrError::Error { error } => {
                assert_eq!(error.code, -32600);
            }
            _ => panic!("Expected error"),
        }
    }

    #[test]
    fn test_mcp_tool_schema_conversion() {
        let tool = McpTool {
            name: "echo".to_string(),
            description: Some("Echo text".to_string()),
            input_schema: serde_json::json!({"type": "object", "properties": {}}),
        };
        assert_eq!(tool.name, "echo");
    }

    #[tokio::test]
    async fn test_mcp_registry_new() {
        let registry = McpServerRegistry::new();
        let names = registry.list_names().await;
        assert!(names.is_empty());
    }
}
