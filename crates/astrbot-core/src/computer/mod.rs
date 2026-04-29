use crate::errors::{AstrBotError, Result};
use async_trait::async_trait;
use once_cell::sync::Lazy;
use regex::Regex;
use std::time::{Duration, Instant};
use tokio::process::Command;
use tokio::time::timeout as tokio_timeout;

/// Result of executing a piece of code.
#[derive(Debug, Clone, PartialEq)]
pub struct ExecutionResult {
    /// Standard output captured from the process.
    pub stdout: String,
    /// Standard error captured from the process.
    pub stderr: String,
    /// Exit code of the process, if available.
    pub exit_code: Option<i32>,
    /// Whether the process exited successfully.
    pub success: bool,
    /// Wall-clock duration of the execution in milliseconds.
    pub duration_ms: u64,
}

/// Trait for code execution backends.
#[async_trait]
pub trait CodeExecutor: Send + Sync {
    /// Unique name of the executor implementation.
    fn name(&self) -> &str;

    /// Execute `code` written in `language` with a `timeout_secs` limit.
    async fn execute(
        &self,
        code: &str,
        language: &str,
        timeout_secs: u64,
    ) -> Result<ExecutionResult>;
}

/// Sandbox configuration for code execution.
///
/// `max_memory_mb` is currently a placeholder documented for future
/// cgroup / rlimit integration.
#[derive(Debug, Clone)]
pub struct ExecutionSandboxConfig {
    /// Maximum allowed memory in megabytes (placeholder).
    pub max_memory_mb: u64,
    /// Whitelist of allowed language identifiers.
    pub allowed_languages: Vec<String>,
    /// Regex patterns that should block execution when matched in code.
    pub blocked_commands: Vec<String>,
    /// Default timeout in seconds when caller does not specify one.
    pub timeout_default: u64,
}

impl Default for ExecutionSandboxConfig {
    fn default() -> Self {
        Self {
            max_memory_mb: 512,
            allowed_languages: vec![
                "python".to_string(),
                "python3".to_string(),
                "py".to_string(),
                "javascript".to_string(),
                "js".to_string(),
                "node".to_string(),
                "bash".to_string(),
                "shell".to_string(),
                "sh".to_string(),
            ],
            blocked_commands: vec![
                r"rm\s+-rf\s+/".to_string(),
                r"mkfs\.".to_string(),
                r"dd\s+if=/dev/zero".to_string(),
                r"dd\s+if=/dev/null".to_string(),
                r"dd\s+if=/dev/random".to_string(),
                r"dd\s+if=/dev/urandom".to_string(),
                r":\(\)\s*\{.*\};\s*:\|:\|".to_string(), // fork bomb :(){ :|:& };:
                r"chmod\s+-R\s+777\s+/".to_string(),
                r">\s*/dev/sda".to_string(),
                r"mv\s+/.*\s+/dev/null".to_string(),
            ],
            timeout_default: 30,
        }
    }
}

// ------------------------------------------------------------------
// Security helpers
// ------------------------------------------------------------------

static DANGEROUS_COMMAND_PATTERNS: Lazy<Vec<Regex>> = Lazy::new(|| {
    ExecutionSandboxConfig::default()
        .blocked_commands
        .iter()
        .filter_map(|p| Regex::new(p).ok())
        .collect()
});

static DANGEROUS_PYTHON_IMPORTS: Lazy<Vec<Regex>> = Lazy::new(|| {
    vec![
        Regex::new(r"(?i)import\s+os\b").unwrap(),
        Regex::new(r"(?i)from\s+os\b").unwrap(),
        Regex::new(r"(?i)import\s+subprocess\b").unwrap(),
        Regex::new(r"(?i)from\s+subprocess\b").unwrap(),
        Regex::new(r"(?i)__import__\s*\(").unwrap(),
        Regex::new(r"(?i)import\s+pty\b").unwrap(),
        Regex::new(r"(?i)import\s+shutil\b").unwrap(),
    ]
});

/// Default executor that runs code as a local subprocess.
pub struct LocalProcessExecutor {
    pub config: ExecutionSandboxConfig,
}

impl LocalProcessExecutor {
    /// Create a new executor with the given sandbox config.
    pub fn new(config: ExecutionSandboxConfig) -> Self {
        Self { config }
    }

    /// Create with default sandbox settings.
    pub fn default_executor() -> Self {
        Self::new(ExecutionSandboxConfig::default())
    }

    /// Validate code against security regexes.
    fn check_security(&self, code: &str, language: &str) -> Result<()> {
        for pattern in DANGEROUS_COMMAND_PATTERNS.iter() {
            if pattern.is_match(code) {
                return Err(AstrBotError::Validation(format!(
                    "blocked command pattern: {}",
                    pattern.as_str()
                )));
            }
        }

        let lang_lower = language.to_lowercase();
        if lang_lower == "python" || lang_lower == "python3" || lang_lower == "py" {
            for pattern in DANGEROUS_PYTHON_IMPORTS.iter() {
                if pattern.is_match(code) {
                    return Err(AstrBotError::Validation(format!(
                        "dangerous Python import: {}",
                        pattern.as_str()
                    )));
                }
            }
        }

        Ok(())
    }

    /// Resolve (interpreter, args) for a given language.
    fn resolve_command(language: &str, code: &str) -> (String, Vec<String>) {
        match language.to_lowercase().as_str() {
            "python" | "python3" | "py" => (
                "python3".to_string(),
                vec!["-c".to_string(), code.to_string()],
            ),
            "javascript" | "js" | "node" => {
                ("node".to_string(), vec!["-e".to_string(), code.to_string()])
            }
            "bash" | "shell" | "sh" => {
                ("bash".to_string(), vec!["-c".to_string(), code.to_string()])
            }
            other => (other.to_string(), vec!["-c".to_string(), code.to_string()]),
        }
    }
}

#[async_trait]
impl CodeExecutor for LocalProcessExecutor {
    fn name(&self) -> &str {
        "local_process"
    }

    async fn execute(
        &self,
        code: &str,
        language: &str,
        timeout_secs: u64,
    ) -> Result<ExecutionResult> {
        // 1. Language validation
        let lang_lower = language.to_lowercase();
        if !self.config.allowed_languages.is_empty()
            && !self
                .config
                .allowed_languages
                .iter()
                .any(|l| l.to_lowercase() == lang_lower)
        {
            return Err(AstrBotError::Validation(format!(
                "language '{}' is not allowed",
                language
            )));
        }

        // 2. Security checks
        self.check_security(code, language)?;

        let (cmd, args) = Self::resolve_command(language, code);
        let timeout_duration = Duration::from_secs(timeout_secs);
        let start = Instant::now();

        // 3. Spawn subprocess
        let child = match Command::new(&cmd)
            .args(&args)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
        {
            Ok(c) => c,
            Err(e) => {
                return Ok(ExecutionResult {
                    stdout: String::new(),
                    stderr: format!("failed to spawn '{}': {}", cmd, e),
                    exit_code: None,
                    success: false,
                    duration_ms: start.elapsed().as_millis() as u64,
                });
            }
        };

        let pid = child.id();

        // 4. Await output with timeout
        let output_fut = async move { child.wait_with_output().await };
        let result = tokio_timeout(timeout_duration, output_fut).await;
        let duration_ms = start.elapsed().as_millis() as u64;

        match result {
            Ok(Ok(output)) => {
                let stdout = String::from_utf8_lossy(&output.stdout).to_string();
                let stderr = String::from_utf8_lossy(&output.stderr).to_string();
                let exit_code = output.status.code();
                let success = output.status.success();

                Ok(ExecutionResult {
                    stdout,
                    stderr,
                    exit_code,
                    success,
                    duration_ms,
                })
            }
            Ok(Err(e)) => Ok(ExecutionResult {
                stdout: String::new(),
                stderr: format!("process error: {}", e),
                exit_code: None,
                success: false,
                duration_ms,
            }),
            Err(_) => {
                // Timeout — attempt to kill by PID
                if let Some(pid) = pid {
                    let _ = Command::new("kill")
                        .args(["-9", &pid.to_string()])
                        .status()
                        .await;
                }

                Ok(ExecutionResult {
                    stdout: String::new(),
                    stderr: "execution timed out".to_string(),
                    exit_code: None,
                    success: false,
                    duration_ms,
                })
            }
        }
    }
}

// ------------------------------------------------------------------
// Plugin wrapper
// ------------------------------------------------------------------

/// Wraps a [`CodeExecutor`] so the plugin system can invoke it as a tool.
pub struct PluginExecutorWrapper {
    executor: Box<dyn CodeExecutor>,
}

impl PluginExecutorWrapper {
    /// Wrap the given executor.
    pub fn new(executor: Box<dyn CodeExecutor>) -> Self {
        Self { executor }
    }

    /// Execute via the wrapped executor.
    pub async fn execute_tool(
        &self,
        code: &str,
        language: &str,
        timeout_secs: u64,
    ) -> Result<ExecutionResult> {
        self.executor.execute(code, language, timeout_secs).await
    }

    /// Exposed name of the wrapped executor.
    pub fn name(&self) -> &str {
        self.executor.name()
    }
}

// ------------------------------------------------------------------
// Tests
// ------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn python_test_executor() -> LocalProcessExecutor {
        // Allow only python3 for focused tests
        let mut cfg = ExecutionSandboxConfig::default();
        cfg.allowed_languages = vec!["python3".to_string()];
        LocalProcessExecutor::new(cfg)
    }

    #[tokio::test]
    async fn test_python_simple_print() {
        let exec = python_test_executor();
        let result = exec
            .execute(r#"print("hello astrbot")"#, "python3", 5)
            .await
            .unwrap();

        assert!(result.success);
        assert_eq!(result.stdout.trim(), "hello astrbot");
        assert!(result.stderr.is_empty());
        assert_eq!(result.exit_code, Some(0));
        assert!(result.duration_ms < 2000);
    }

    #[tokio::test]
    async fn test_timeout_handling() {
        let exec = python_test_executor();
        let result = exec
            .execute(
                "import time; time.sleep(10)",
                "python3",
                1, // 1-second timeout
            )
            .await
            .unwrap();

        assert!(!result.success);
        assert!(
            result.stderr.contains("timed out"),
            "stderr was: {}",
            result.stderr
        );
        assert!(result.stdout.is_empty());
        assert_eq!(result.exit_code, None);
    }

    #[tokio::test]
    async fn test_blocked_command_rejection() {
        let exec = python_test_executor();
        let err = exec.execute("rm -rf /", "python3", 5).await.unwrap_err();

        match err {
            AstrBotError::Validation(msg) => {
                assert!(msg.contains("blocked"), "msg was: {}", msg);
            }
            other => panic!("expected Validation error, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_dangerous_python_import_rejection() {
        let exec = python_test_executor();
        let err = exec
            .execute("import os; os.system('ls')", "python3", 5)
            .await
            .unwrap_err();

        match err {
            AstrBotError::Validation(msg) => {
                assert!(msg.contains("dangerous"), "msg was: {}", msg);
            }
            other => panic!("expected Validation error, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_invalid_language_handling() {
        let exec = python_test_executor();
        let err = exec.execute("echo hello", "rust", 5).await.unwrap_err();

        match err {
            AstrBotError::Validation(msg) => {
                assert!(msg.contains("not allowed"), "msg was: {}", msg);
            }
            other => panic!("expected Validation error, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_bash_execution_not_allowed_by_default_config() {
        // Default config allows bash, but our test helper only allows python3
        let exec = python_test_executor();
        let err = exec.execute("echo hello", "bash", 5).await.unwrap_err();

        match err {
            AstrBotError::Validation(msg) => {
                assert!(msg.contains("not allowed"));
            }
            other => panic!("expected Validation error, got {:?}", other),
        }
    }
}

pub mod booter;
