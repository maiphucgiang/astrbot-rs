pub mod ast_checker;

use anyhow::{bail, Result};
use std::process::Stdio;
use std::time::Duration;
use tokio::process::Command;
use tokio::time::timeout;

/// 执行结果，强制限定返回类型
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct SafeExecutionResult {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
}

pub struct HardenedLocalExecutor {
    pub max_cpu_secs: u64,
    pub max_mem_mb: u64,
    pub max_output_bytes: usize,
    pub allowed_env_vars: Vec<String>,
}

impl Default for HardenedLocalExecutor {
    fn default() -> Self {
        Self {
            max_cpu_secs: 5,
            max_mem_mb: 128,
            max_output_bytes: 64 * 1024,
            allowed_env_vars: vec![
                "PATH".to_string(),
                "PYTHONDONTWRITEBYTECODE".to_string(),
                "PYTHONUNBUFFERED".to_string(),
                "LANG".to_string(),
            ],
        }
    }
}

impl HardenedLocalExecutor {
    pub async fn execute(&self, code: &str) -> Result<SafeExecutionResult> {
        ast_checker::AstChecker::check(code)?;

        let mut cmd = self.build_sandbox_command(code)?;

        let child = cmd.stdout(Stdio::piped()).stderr(Stdio::piped()).spawn()?;

        let result = timeout(
            Duration::from_secs(self.max_cpu_secs + 2),
            child.wait_with_output(),
        )
        .await;

        match result {
            Ok(Ok(output)) => {
                let stdout = String::from_utf8_lossy(&output.stdout)
                    .chars()
                    .take(self.max_output_bytes)
                    .collect();
                let stderr = String::from_utf8_lossy(&output.stderr)
                    .chars()
                    .take(self.max_output_bytes)
                    .collect();

                Ok(SafeExecutionResult {
                    stdout,
                    stderr,
                    exit_code: output.status.code().unwrap_or(-1),
                })
            }
            Ok(Err(e)) => bail!("Execution failed: {}", e),
            Err(_) => bail!(
                "Execution timed out after {}s (CPU limit exceeded)",
                self.max_cpu_secs
            ),
        }
    }

    #[cfg(target_os = "linux")]
    fn build_sandbox_command(&self, code: &str) -> Result<Command> {
        let bwrap_available = std::process::Command::new("which")
            .arg("bwrap")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);

        let mut cmd = if bwrap_available {
            let mut c = Command::new("bwrap");
            c.arg("--unshare-net")
                .arg("--unshare-pid")
                .arg("--die-with-parent")
                .arg("--ro-bind")
                .arg("/usr")
                .arg("/usr")
                .arg("--ro-bind")
                .arg("/lib")
                .arg("/lib")
                .arg("--ro-bind")
                .arg("/lib64")
                .arg("/lib64")
                .arg("--tmpfs")
                .arg("/tmp")
                .arg("--tmpfs")
                .arg("/home")
                .arg("--proc")
                .arg("/proc")
                .arg("--dev")
                .arg("/dev")
                .arg("--chdir")
                .arg("/tmp")
                .arg("python3");
            c
        } else {
            Command::new("python3")
        };

        self.apply_python_args(&mut cmd, code);
        Ok(cmd)
    }

    #[cfg(target_os = "macos")]
    fn build_sandbox_command(&self, code: &str) -> Result<Command> {
        let profile = r#"
(version 1)
(deny default)
(allow process-exec (subpath "/usr/bin/python3"))
(allow process-exec (subpath "/usr/local/bin/python3"))
(allow file-read* (subpath "/usr"))
(allow file-read* (subpath "/System"))
(allow file-read* (subpath "/dev"))
(allow file-write* (subpath "/tmp"))
(deny network*)
"#;
        let profile_path = "/tmp/astrbot_sandbox.sb";
        let _ = std::fs::write(profile_path, profile);

        let mut cmd = Command::new("sandbox-exec");
        cmd.arg("-f").arg(profile_path).arg("python3");
        self.apply_python_args(&mut cmd, code);
        Ok(cmd)
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    fn build_sandbox_command(&self, code: &str) -> Result<Command> {
        let mut cmd = Command::new("python3");
        self.apply_python_args(&mut cmd, code);
        Ok(cmd)
    }

    fn apply_python_args(&self, cmd: &mut Command, code: &str) {
        cmd.arg("-I").arg("-S").arg("-c").arg(code);
        cmd.env_clear();
        for key in &self.allowed_env_vars {
            if let Ok(val) = std::env::var(key) {
                cmd.env(key, val);
            }
        }
        cmd.env("PYTHONDONTWRITEBYTECODE", "1");
        cmd.env("PYTHONUNBUFFERED", "1");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_safe_execution() {
        let exec = HardenedLocalExecutor::default();
        let result = exec.execute("print('hello')").await.unwrap();
        assert!(result.stdout.contains("hello"));
    }

    #[tokio::test]
    async fn test_forbidden_code_blocked() {
        let exec = HardenedLocalExecutor::default();
        let result = exec.execute("import os; os.system('id')").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_timeout() {
        let exec = HardenedLocalExecutor {
            max_cpu_secs: 1,
            ..Default::default()
        };
        let result = exec.execute("import time; time.sleep(100)").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("timed out"));
    }
}
