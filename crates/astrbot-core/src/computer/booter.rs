use anyhow::Result;
use astrbot_security::executor::{HardenedLocalExecutor, SafeExecutionResult};
use std::path::Path;
use tracing::{error, info, warn};

/// Result of a shell command execution in a sandbox
#[derive(Debug, Clone)]
pub struct ShellResult {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: Option<i32>,
    pub success: bool,
}

impl From<SafeExecutionResult> for ShellResult {
    fn from(r: SafeExecutionResult) -> Self {
        Self {
            stdout: r.stdout,
            stderr: r.stderr,
            exit_code: Some(r.exit_code),
            success: r.exit_code == 0,
        }
    }
}

/// Abstract interface for a Computer Use sandbox
#[async_trait::async_trait]
pub trait ComputerBooter: Send + Sync {
    /// Boot the sandbox environment
    async fn boot(&self, session_id: &str) -> Result<()>;

    /// Shutdown and optionally delete the sandbox
    async fn shutdown(&self, delete_sandbox: bool) -> Result<()>;

    /// Execute a shell command in the sandbox
    async fn exec(&self, command: &str) -> Result<ShellResult>;

    /// Upload a file to the sandbox
    async fn upload_file(&self, local_path: &str, remote_path: &str) -> Result<serde_json::Value>;

    /// Download a file from the sandbox
    async fn download_file(&self, remote_path: &str, local_path: &str) -> Result<()>;

    /// Check if the sandbox is available
    async fn available(&self) -> bool;
}

/// Local sandbox — runs commands directly on the host with hardening
pub struct LocalBooter {
    executor: HardenedLocalExecutor,
    working_dir: String,
}

impl LocalBooter {
    pub fn new() -> Self {
        Self {
            executor: HardenedLocalExecutor::default(),
            working_dir: "/tmp/astrbot_computer".to_string(),
        }
    }

    pub fn with_limits(cpu_secs: u64, mem_mb: u64) -> Self {
        Self {
            executor: HardenedLocalExecutor {
                max_cpu_secs: cpu_secs,
                max_mem_mb: mem_mb,
                ..Default::default()
            },
            working_dir: "/tmp/astrbot_computer".to_string(),
        }
    }
}

#[async_trait::async_trait]
impl ComputerBooter for LocalBooter {
    async fn boot(&self, _session_id: &str) -> Result<()> {
        tokio::fs::create_dir_all(&self.working_dir).await?;
        info!("[Computer] Local sandbox booted at {}", self.working_dir);
        Ok(())
    }

    async fn shutdown(&self, delete_sandbox: bool) -> Result<()> {
        if delete_sandbox {
            let _ = tokio::fs::remove_dir_all(&self.working_dir).await;
        }
        info!("[Computer] Local sandbox shutdown");
        Ok(())
    }

    async fn exec(&self, command: &str) -> Result<ShellResult> {
        let result = self.executor.execute(command).await?;
        Ok(result.into())
    }

    async fn upload_file(&self, local_path: &str, remote_path: &str) -> Result<serde_json::Value> {
        let content = tokio::fs::read(local_path).await?;
        let dest = format!("{}/{}", self.working_dir, remote_path);
        if let Some(parent) = std::path::Path::new(&dest).parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        tokio::fs::write(&dest, content).await?;
        info!("[Computer] Uploaded {} -> {}", local_path, dest);
        Ok(serde_json::json!({ "success": true, "remote_path": dest }))
    }

    async fn download_file(&self, remote_path: &str, local_path: &str) -> Result<()> {
        let src = format!("{}/{}", self.working_dir, remote_path);
        let content = tokio::fs::read(&src).await?;
        tokio::fs::write(local_path, content).await?;
        info!("[Computer] Downloaded {} -> {}", src, local_path);
        Ok(())
    }

    async fn available(&self) -> bool {
        true
    }
}

/// Session-scoped booter cache
pub struct BooterManager {
    booters: dashmap::DashMap<String, std::sync::Arc<dyn ComputerBooter>>,
}

impl BooterManager {
    pub fn new() -> Self {
        Self {
            booters: dashmap::DashMap::new(),
        }
    }

    pub async fn get_or_create(&self, session_id: &str) -> Result<std::sync::Arc<dyn ComputerBooter>> {
        if let Some(entry) = self.booters.get(session_id) {
            if entry.value().available().await {
                return Ok(std::sync::Arc::clone(entry.value()));
            }
        }
        self.booters.remove(session_id);
        
        let booter: std::sync::Arc<dyn ComputerBooter> = std::sync::Arc::new(LocalBooter::new());
        booter.boot(session_id).await?;
        self.booters.insert(session_id.to_string(), std::sync::Arc::clone(&booter));
        Ok(booter)
    }

    pub async fn sync_skills(&self, session_id: &str, skills_root: &str) -> Result<()> {
        let booter = self.get_or_create(session_id).await?;
        let zip_path = format!("{}/skills_bundle.zip", skills_root);
        if std::path::Path::new(&zip_path).exists() {
            booter.upload_file(&zip_path, "skills.zip").await?;
        }
        let scan_result = booter.exec("import json, os; skills=[]; [skills.append({'name': d}) for d in os.listdir('/tmp/astrbot_computer') if os.path.isdir(d)]; print(json.dumps(skills))").await?;
        info!("[Computer] Skills sync for {}: {}", session_id, scan_result.stdout);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_local_booter_boot_shutdown() {
        let booter = LocalBooter::new();
        booter.boot("test_session").await.unwrap();
        assert!(booter.available().await);
        booter.shutdown(true).await.unwrap();
    }

    #[tokio::test]
    async fn test_local_booter_exec_python() {
        // Skip if Python is not available in the test environment.
        let has_python = std::process::Command::new("python3")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
            || std::process::Command::new("python")
                .arg("--version")
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false);
        if !has_python {
            return;
        }

        let booter = LocalBooter::new();
        booter.boot("test_exec").await.unwrap();
        let result = booter.exec("print('hello computer')").await.unwrap();
        assert!(result.success);
        assert!(result.stdout.contains("hello computer"));
        booter.shutdown(true).await.unwrap();
    }

    #[tokio::test]
    async fn test_booter_manager_get_or_create() {
        let mgr = BooterManager::new();
        let booter = mgr.get_or_create("session_1").await.unwrap();
        assert!(booter.available().await);
        let booter2 = mgr.get_or_create("session_1").await.unwrap();
        assert!(booter2.available().await);
    }
}