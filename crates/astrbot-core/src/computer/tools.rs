//! Computer Use Agent (CUA) toolset

use async_trait::async_trait;
use serde_json::{json, Value};
use std::path::Path;
use tokio::fs;
use tokio::process::Command;

use crate::errors::{AstrBotError, Result};
use crate::tools::{Tool, ToolDefinition, ToolParameter, ToolResult};

// ── FsTool ──

pub struct FsTool {
    definition: ToolDefinition,
    work_dir: String,
}

impl FsTool {
    pub fn new(work_dir: impl Into<String>) -> Self {
        Self {
            definition: ToolDefinition {
                name: "fs".to_string(),
                description: "File-system operations: read, write, list, grep, find".to_string(),
                parameters: vec![
                    ToolParameter {
                        name: "action".to_string(),
                        description: "read | write | list | grep | find".to_string(),
                        param_type: "string".to_string(),
                        required: true,
                        default: None,
                        enum_values: Some(vec![
                            "read".to_string(),
                            "write".to_string(),
                            "list".to_string(),
                            "grep".to_string(),
                            "find".to_string(),
                        ]),
                    },
                    ToolParameter {
                        name: "path".to_string(),
                        description: "Target path".to_string(),
                        param_type: "string".to_string(),
                        required: true,
                        default: None,
                        enum_values: None,
                    },
                    ToolParameter {
                        name: "content".to_string(),
                        description: "Text for write".to_string(),
                        param_type: "string".to_string(),
                        required: false,
                        default: None,
                        enum_values: None,
                    },
                    ToolParameter {
                        name: "pattern".to_string(),
                        description: "Regex for grep/find".to_string(),
                        param_type: "string".to_string(),
                        required: false,
                        default: None,
                        enum_values: None,
                    },
                    ToolParameter {
                        name: "max_depth".to_string(),
                        description: "Recursion depth (default 10)".to_string(),
                        param_type: "number".to_string(),
                        required: false,
                        default: Some(json!(10)),
                        enum_values: None,
                    },
                ],
                returns: Some("string".to_string()),
                requires_confirmation: false,
            },
            work_dir: work_dir.into(),
        }
    }

    pub fn resolve_path(&self, input: &str) -> std::path::PathBuf {
        let input = input.trim_start_matches('/');
        std::path::Path::new(&self.work_dir).join(input)
    }

    pub async fn action_read(&self, path: &Path) -> Result<ToolResult> {
        if !path.exists() {
            return Ok(ToolResult::Error {
                message: format!("File not found: {}", path.display()),
            });
        }
        let content = fs::read_to_string(path)
            .await
            .map_err(|e| AstrBotError::Internal(format!("Read failed: {}", e)))?;
        Ok(ToolResult::Success {
            output: Value::String(content),
        })
    }

    pub async fn action_write(&self, path: &Path, content: &str) -> Result<ToolResult> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .await
                .map_err(|e| AstrBotError::Internal(format!("mkdir failed: {}", e)))?;
        }
        fs::write(path, content)
            .await
            .map_err(|e| AstrBotError::Internal(format!("Write failed: {}", e)))?;
        Ok(ToolResult::Success {
            output: Value::String(format!(
                "Wrote {} bytes to {}",
                content.len(),
                path.display()
            )),
        })
    }

    pub async fn action_list(&self, path: &Path, max_depth: usize) -> Result<ToolResult> {
        if !path.exists() {
            return Ok(ToolResult::Error {
                message: format!("Dir not found: {}", path.display()),
            });
        }
        let mut entries = Vec::new();
        self.walk_dir(path, max_depth, 0, &mut entries).await?;
        Ok(ToolResult::Success {
            output: Value::Array(entries),
        })
    }

    async fn walk_dir(
        &self,
        dir: &Path,
        max_depth: usize,
        depth: usize,
        out: &mut Vec<Value>,
    ) -> Result<()> {
        if depth > max_depth {
            return Ok(());
        }
        let mut entries = fs::read_dir(dir)
            .await
            .map_err(|e| AstrBotError::Internal(format!("read_dir: {}", e)))?;
        while let Some(entry) = entries
            .next_entry()
            .await
            .map_err(|e| AstrBotError::Internal(format!("entry: {}", e)))?
        {
            let meta = entry.metadata().await.ok();
            let is_dir = meta.as_ref().map(|m| m.is_dir()).unwrap_or(false);
            let size = meta.as_ref().map(|m| m.len()).unwrap_or(0);
            let name = entry.file_name().to_string_lossy().to_string();
            let full = entry.path();
            out.push(json!({"name": name, "path": full.to_string_lossy(), "is_dir": is_dir, "size": size, "depth": depth}));
            if is_dir {
                Box::pin(self.walk_dir(&full, max_depth, depth + 1, out)).await?;
            }
        }
        Ok(())
    }

    pub async fn action_grep(
        &self,
        dir: &Path,
        pattern: &str,
        max_depth: usize,
    ) -> Result<ToolResult> {
        if !dir.exists() {
            return Ok(ToolResult::Error {
                message: format!("Dir not found: {}", dir.display()),
            });
        }
        let re = regex::Regex::new(pattern)
            .map_err(|e| AstrBotError::Validation(format!("Invalid regex: {}", e)))?;
        let mut matches = Vec::new();
        self.grep_dir(dir, &re, max_depth, 0, &mut matches).await?;
        Ok(ToolResult::Success {
            output: Value::Array(matches),
        })
    }

    async fn grep_dir(
        &self,
        dir: &Path,
        re: &regex::Regex,
        max_depth: usize,
        depth: usize,
        out: &mut Vec<Value>,
    ) -> Result<()> {
        if depth > max_depth {
            return Ok(());
        }
        let mut entries = fs::read_dir(dir)
            .await
            .map_err(|e| AstrBotError::Internal(format!("read_dir: {}", e)))?;
        while let Some(entry) = entries
            .next_entry()
            .await
            .map_err(|e| AstrBotError::Internal(format!("entry: {}", e)))?
        {
            let path = entry.path();
            let meta = entry.metadata().await.ok();
            if meta.as_ref().map(|m| m.is_dir()).unwrap_or(false) {
                Box::pin(self.grep_dir(&path, re, max_depth, depth + 1, out)).await?;
            } else if meta.as_ref().map(|m| m.is_file()).unwrap_or(false) {
                if let Ok(content) = fs::read_to_string(&path).await {
                    for (line_no, line) in content.lines().enumerate() {
                        if re.is_match(line) {
                            out.push(json!({"file": path.to_string_lossy(), "line": line_no + 1, "text": line}));
                        }
                    }
                }
            }
        }
        Ok(())
    }

    pub async fn action_find(
        &self,
        dir: &Path,
        pattern: &str,
        max_depth: usize,
    ) -> Result<ToolResult> {
        if !dir.exists() {
            return Ok(ToolResult::Error {
                message: format!("Dir not found: {}", dir.display()),
            });
        }
        let mut results = Vec::new();
        self.find_dir(dir, pattern, max_depth, 0, &mut results)
            .await?;
        Ok(ToolResult::Success {
            output: Value::Array(results),
        })
    }

    async fn find_dir(
        &self,
        dir: &Path,
        pattern: &str,
        max_depth: usize,
        depth: usize,
        out: &mut Vec<Value>,
    ) -> Result<()> {
        if depth > max_depth {
            return Ok(());
        }
        let mut entries = fs::read_dir(dir)
            .await
            .map_err(|e| AstrBotError::Internal(format!("read_dir: {}", e)))?;
        while let Some(entry) = entries
            .next_entry()
            .await
            .map_err(|e| AstrBotError::Internal(format!("entry: {}", e)))?
        {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            let meta = entry.metadata().await.ok();
            let is_dir = meta.as_ref().map(|m| m.is_dir()).unwrap_or(false);
            if name.contains(pattern) {
                out.push(json!({"name": name, "path": path.to_string_lossy(), "is_dir": is_dir, "depth": depth}));
            }
            if is_dir {
                Box::pin(self.find_dir(&path, pattern, max_depth, depth + 1, out)).await?;
            }
        }
        Ok(())
    }
}

#[async_trait]
impl Tool for FsTool {
    fn definition(&self) -> &ToolDefinition {
        &self.definition
    }
    async fn execute(&self, arguments: &Value) -> Result<ToolResult> {
        let action = arguments
            .get("action")
            .and_then(|v| v.as_str())
            .unwrap_or("read");
        let path = arguments
            .get("path")
            .and_then(|v| v.as_str())
            .unwrap_or(".");
        let resolved = self.resolve_path(path);
        let max_depth = arguments
            .get("max_depth")
            .and_then(|v| v.as_u64())
            .map(|n| n as usize)
            .unwrap_or(10);
        match action {
            "read" => self.action_read(&resolved).await,
            "write" => {
                self.action_write(
                    &resolved,
                    arguments
                        .get("content")
                        .and_then(|v| v.as_str())
                        .unwrap_or(""),
                )
                .await
            }
            "list" => self.action_list(&resolved, max_depth).await,
            "grep" => {
                self.action_grep(
                    &resolved,
                    arguments
                        .get("pattern")
                        .and_then(|v| v.as_str())
                        .unwrap_or(".*"),
                    max_depth,
                )
                .await
            }
            "find" => {
                self.action_find(
                    &resolved,
                    arguments
                        .get("pattern")
                        .and_then(|v| v.as_str())
                        .unwrap_or(""),
                    max_depth,
                )
                .await
            }
            other => Ok(ToolResult::Error {
                message: format!("Unknown fs action: {}", other),
            }),
        }
    }
}

// ── PythonShellTool ──

pub struct PythonShellTool {
    definition: ToolDefinition,
    work_dir: String,
}

impl PythonShellTool {
    pub fn new(work_dir: impl Into<String>) -> Self {
        Self {
            definition: ToolDefinition {
                name: "python_shell".to_string(),
                description: "Execute Python or shell commands. language='python'|'shell', code='...', timeout_secs=30".to_string(),
                parameters: vec![
                    ToolParameter { name: "language".to_string(), description: "python or shell".to_string(), param_type: "string".to_string(), required: true, default: Some(json!("python")), enum_values: Some(vec!["python".to_string(), "shell".to_string()]) },
                    ToolParameter { name: "code".to_string(), description: "Code to run".to_string(), param_type: "string".to_string(), required: true, default: None, enum_values: None },
                    ToolParameter { name: "timeout_secs".to_string(), description: "Timeout (default 30, max 120)".to_string(), param_type: "number".to_string(), required: false, default: Some(json!(30)), enum_values: None },
                ],
                returns: Some("string".to_string()),
                requires_confirmation: true,
            },
            work_dir: work_dir.into(),
        }
    }

    pub async fn run_python(&self, code: &str, timeout_secs: u64) -> Result<ToolResult> {
        let start = std::time::Instant::now();
        let output = tokio::time::timeout(
            std::time::Duration::from_secs(timeout_secs),
            Command::new("python3")
                .arg("-c")
                .arg(code)
                .current_dir(&self.work_dir)
                .output(),
        )
        .await;
        let duration_ms = start.elapsed().as_millis() as u64;
        match output {
            Ok(Ok(out)) => {
                let stdout = String::from_utf8_lossy(&out.stdout).to_string();
                let stderr = String::from_utf8_lossy(&out.stderr).to_string();
                Ok(ToolResult::Success {
                    output: json!({"stdout": stdout, "stderr": stderr, "exit_code": out.status.code(), "success": out.status.success(), "duration_ms": duration_ms}),
                })
            }
            Ok(Err(e)) => Ok(ToolResult::Error {
                message: format!("spawn failed: {}", e),
            }),
            Err(_) => Ok(ToolResult::Error {
                message: format!("Timed out after {}s", timeout_secs),
            }),
        }
    }

    pub async fn run_shell(&self, code: &str, timeout_secs: u64) -> Result<ToolResult> {
        let start = std::time::Instant::now();
        let output = tokio::time::timeout(
            std::time::Duration::from_secs(timeout_secs),
            Command::new("bash")
                .arg("-c")
                .arg(code)
                .current_dir(&self.work_dir)
                .output(),
        )
        .await;
        let duration_ms = start.elapsed().as_millis() as u64;
        match output {
            Ok(Ok(out)) => {
                let stdout = String::from_utf8_lossy(&out.stdout).to_string();
                let stderr = String::from_utf8_lossy(&out.stderr).to_string();
                Ok(ToolResult::Success {
                    output: json!({"stdout": stdout, "stderr": stderr, "exit_code": out.status.code(), "success": out.status.success(), "duration_ms": duration_ms}),
                })
            }
            Ok(Err(e)) => Ok(ToolResult::Error {
                message: format!("spawn failed: {}", e),
            }),
            Err(_) => Ok(ToolResult::Error {
                message: format!("Timed out after {}s", timeout_secs),
            }),
        }
    }
}

#[async_trait]
impl Tool for PythonShellTool {
    fn definition(&self) -> &ToolDefinition {
        &self.definition
    }
    async fn execute(&self, arguments: &Value) -> Result<ToolResult> {
        let language = arguments
            .get("language")
            .and_then(|v| v.as_str())
            .unwrap_or("python");
        let code = arguments
            .get("code")
            .and_then(|v| v.as_str())
            .ok_or_else(|| AstrBotError::Validation("Missing code".to_string()))?;
        let timeout_secs = arguments
            .get("timeout_secs")
            .and_then(|v| v.as_u64())
            .map(|n| n.min(120))
            .unwrap_or(30);
        match language {
            "python" => self.run_python(code, timeout_secs).await,
            "shell" => self.run_shell(code, timeout_secs).await,
            other => Ok(ToolResult::Error {
                message: format!("Unsupported: {}", other),
            }),
        }
    }
}

// ── ComputerUseTool — unified CUA ──

pub struct ComputerUseTool {
    definition: ToolDefinition,
    fs_tool: FsTool,
    shell_tool: PythonShellTool,
}

impl ComputerUseTool {
    pub fn new(work_dir: impl Into<String>) -> Self {
        let dir: String = work_dir.into();
        Self {
            definition: ToolDefinition {
                name: "computer_use".to_string(),
                description: "Unified computer-use: read_file, write_file, execute_python, execute_shell, list_directory, search_files".to_string(),
                parameters: vec![
                    ToolParameter { name: "command".to_string(), description: "read_file | write_file | execute_python | execute_shell | list_directory | search_files".to_string(), param_type: "string".to_string(), required: true, default: None, enum_values: Some(vec!["read_file".to_string(), "write_file".to_string(), "execute_python".to_string(), "execute_shell".to_string(), "list_directory".to_string(), "search_files".to_string()]) },
                    ToolParameter { name: "path".to_string(), description: "File or dir path".to_string(), param_type: "string".to_string(), required: false, default: None, enum_values: None },
                    ToolParameter { name: "content".to_string(), description: "Content for write_file".to_string(), param_type: "string".to_string(), required: false, default: None, enum_values: None },
                    ToolParameter { name: "code".to_string(), description: "Code for execute_*".to_string(), param_type: "string".to_string(), required: false, default: None, enum_values: None },
                    ToolParameter { name: "pattern".to_string(), description: "Pattern for search_files".to_string(), param_type: "string".to_string(), required: false, default: None, enum_values: None },
                ],
                returns: Some("string".to_string()),
                requires_confirmation: true,
            },
            fs_tool: FsTool::new(dir.clone()),
            shell_tool: PythonShellTool::new(dir),
        }
    }
}

#[async_trait]
impl Tool for ComputerUseTool {
    fn definition(&self) -> &ToolDefinition {
        &self.definition
    }
    async fn execute(&self, arguments: &Value) -> Result<ToolResult> {
        let command = arguments
            .get("command")
            .and_then(|v| v.as_str())
            .unwrap_or("read_file");
        match command {
            "read_file" => {
                self.fs_tool
                    .action_read(
                        &self.fs_tool.resolve_path(
                            arguments
                                .get("path")
                                .and_then(|v| v.as_str())
                                .unwrap_or("."),
                        ),
                    )
                    .await
            }
            "write_file" => {
                self.fs_tool
                    .action_write(
                        &self.fs_tool.resolve_path(
                            arguments
                                .get("path")
                                .and_then(|v| v.as_str())
                                .unwrap_or("."),
                        ),
                        arguments
                            .get("content")
                            .and_then(|v| v.as_str())
                            .unwrap_or(""),
                    )
                    .await
            }
            "list_directory" => {
                self.fs_tool
                    .action_list(
                        &self.fs_tool.resolve_path(
                            arguments
                                .get("path")
                                .and_then(|v| v.as_str())
                                .unwrap_or("."),
                        ),
                        10,
                    )
                    .await
            }
            "search_files" => {
                self.fs_tool
                    .action_grep(
                        &self.fs_tool.resolve_path(
                            arguments
                                .get("path")
                                .and_then(|v| v.as_str())
                                .unwrap_or("."),
                        ),
                        arguments
                            .get("pattern")
                            .and_then(|v| v.as_str())
                            .unwrap_or(".*"),
                        10,
                    )
                    .await
            }
            "execute_python" => {
                self.shell_tool
                    .run_python(
                        arguments.get("code").and_then(|v| v.as_str()).unwrap_or(""),
                        30,
                    )
                    .await
            }
            "execute_shell" => {
                self.shell_tool
                    .run_shell(
                        arguments.get("code").and_then(|v| v.as_str()).unwrap_or(""),
                        30,
                    )
                    .await
            }
            other => Ok(ToolResult::Error {
                message: format!("Unknown: {}", other),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::fs;

    fn temp_work_dir() -> String {
        let dir = std::env::temp_dir().join(format!("cua_test_{}", std::process::id()));
        let s = dir.to_string_lossy().to_string();
        std::fs::create_dir_all(&dir).ok();
        s
    }

    #[tokio::test]
    async fn test_fs_read_and_write() {
        let work = temp_work_dir();
        let tool = FsTool::new(&work);
        let w = tool
            .execute(&json!({"action": "write", "path": "test.txt", "content": "hello world"}))
            .await
            .unwrap();
        assert!(matches!(w, ToolResult::Success { .. }));
        let r = tool
            .execute(&json!({"action": "read", "path": "test.txt"}))
            .await
            .unwrap();
        match r {
            ToolResult::Success { output } => assert_eq!(output.as_str().unwrap(), "hello world"),
            _ => panic!("Expected success"),
        }
        let _ = fs::remove_dir_all(&work).await;
    }

    #[tokio::test]
    async fn test_fs_list_and_find() {
        let work = temp_work_dir();
        let tool = FsTool::new(&work);
        let sub = std::path::Path::new(&work).join("sub");
        fs::create_dir(&sub).await.unwrap();
        fs::write(sub.join("a.rs"), "fn main(){}").await.unwrap();
        fs::write(sub.join("b.txt"), "hello").await.unwrap();
        let l = tool
            .execute(&json!({"action": "list", "path": ".", "max_depth": 2}))
            .await
            .unwrap();
        assert!(matches!(l, ToolResult::Success { .. }));
        if let ToolResult::Success { output } = l {
            let names: Vec<&str> = output
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v["name"].as_str().unwrap())
                .collect();
            assert!(names.contains(&"sub"));
            assert!(names.contains(&"a.rs"));
            assert!(names.contains(&"b.txt"));
        }
        let f = tool
            .execute(&json!({"action": "find", "path": ".", "pattern": ".rs"}))
            .await
            .unwrap();
        if let ToolResult::Success { output } = f {
            assert_eq!(output.as_array().unwrap().len(), 1);
        }
        let _ = fs::remove_dir_all(&work).await;
    }

    #[tokio::test]
    async fn test_computer_use_write_and_read() {
        let work = temp_work_dir();
        let tool = ComputerUseTool::new(&work);
        let w = tool.execute(&json!({"command": "write_file", "path": "data.json", "content": "{\"key\": \"value\"}"})).await.unwrap();
        assert!(matches!(w, ToolResult::Success { .. }));
        let r = tool
            .execute(&json!({"command": "read_file", "path": "data.json"}))
            .await
            .unwrap();
        match r {
            ToolResult::Success { output } => {
                assert_eq!(output.as_str().unwrap(), "{\"key\": \"value\"}")
            }
            _ => panic!("Expected success"),
        }
        let _ = fs::remove_dir_all(&work).await;
    }

    #[tokio::test]
    async fn test_computer_use_list_directory() {
        let work = temp_work_dir();
        let tool = ComputerUseTool::new(&work);
        fs::write(std::path::Path::new(&work).join("foo.txt"), "x")
            .await
            .unwrap();
        let r = tool
            .execute(&json!({"command": "list_directory", "path": "."}))
            .await
            .unwrap();
        assert!(matches!(r, ToolResult::Success { .. }));
        let _ = fs::remove_dir_all(&work).await;
    }

    #[tokio::test]
    async fn test_python_shell_timeout() {
        let work = temp_work_dir();
        let tool = PythonShellTool::new(&work);
        let r = tool.execute(&json!({"language": "python", "code": "import time; time.sleep(5)", "timeout_secs": 1})).await.unwrap();
        assert!(matches!(r, ToolResult::Error { ref message } if message.contains("Timed out")));
        let _ = fs::remove_dir_all(&work).await;
    }

    #[tokio::test]
    async fn test_fs_grep() {
        let work = temp_work_dir();
        let tool = FsTool::new(&work);
        fs::write(
            std::path::Path::new(&work).join("src.txt"),
            "apple
banana
apple pie
cherry",
        )
        .await
        .unwrap();
        let r = tool
            .execute(&json!({"action": "grep", "path": ".", "pattern": "apple"}))
            .await
            .unwrap();
        match r {
            ToolResult::Success { output } => {
                let arr = output.as_array().unwrap();
                assert_eq!(arr.len(), 2);
            }
            _ => panic!("Expected success"),
        }
        let _ = fs::remove_dir_all(&work).await;
    }
}
