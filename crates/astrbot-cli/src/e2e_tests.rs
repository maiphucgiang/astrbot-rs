//! E2E integration tests for AstrBot CLI
//!
//! These tests invoke the compiled `astrbot` binary via `std::process::Command`
//! and verify real CLI behavior end-to-end.

use std::fs;
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Command, Stdio};

/// Path to the compiled `astrbot` debug binary.
fn astrbot_bin() -> PathBuf {
    // Workspace root is two levels up from crates/astrbot-cli
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.pop(); // crates
    path.pop(); // workspace root
    path.push("target");
    path.push("debug");
    path.push("astrbot");
    path
}

/// Build the `astrbot` binary if it doesn't exist or is stale.
/// This is a best-effort helper; CI should build beforehand.
fn ensure_binary_built() {
    let bin = astrbot_bin();
    if bin.exists() {
        return;
    }

    let mut workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    workspace_root.pop();
    workspace_root.pop();

    let status = Command::new("cargo")
        .args(["build", "--bin", "astrbot"])
        .current_dir(&workspace_root)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .expect("cargo build should be available");

    assert!(status.success(), "cargo build --bin astrbot failed");
    assert!(bin.exists(), "binary should exist after build");
}

/// Generate a minimal valid AstrBot YAML config.
fn minimal_config_yaml(port: u16, db_path: &str) -> String {
    format!(
        r#"nickname: "TestBot"
prefixes:
  - "/"
admins: []
platforms: []
providers: []
plugins: {{}}
webui:
  enabled: true
  host: "127.0.0.1"
  port: {}
  jwt_secret: "test-secret"
  tls_cert: null
  tls_key: null
log_level: "info"
database_url: "sqlite:{}"
"#,
        port, db_path
    )
}

/// Pick a random available TCP port on 127.0.0.1.
fn find_free_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind to random port");
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    port
}

// ============================================================================
// Test 1: `astrbot init` creates a valid config file
// ============================================================================
#[test]
fn test_e2e_init_config() {
    ensure_binary_built();

    let temp_dir = tempfile::tempdir().expect("create temp dir");
    let config_path = temp_dir.path().join("astrbot_config.yaml");

    let output = Command::new(astrbot_bin())
        .args(["init", "--dir", temp_dir.path().to_str().unwrap()])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("spawn astrbot init");

    assert!(
        output.status.success(),
        "astrbot init should exit 0. stdout: {}, stderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    assert!(
        config_path.exists(),
        "config file should be created at {:?}",
        config_path
    );

    // Verify the generated file is valid YAML and non-empty
    let content = fs::read_to_string(&config_path).expect("read config file");
    assert!(!content.is_empty(), "config file should not be empty");
    assert!(content.contains("nickname"), "config should contain nickname field");

    // Verify the file can be parsed by the core config loader
    // We do this via a runtime check: `astrbot validate` should return 0
    let validate_output = Command::new(astrbot_bin())
        .args([
            "validate",
            "--config",
            config_path.to_str().unwrap(),
        ])
        .env("RUST_LOG", "info")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("spawn astrbot validate");

    assert!(
        validate_output.status.success(),
        "astrbot validate should succeed on init-generated config. stdout: {}, stderr: {}",
        String::from_utf8_lossy(&validate_output.stdout),
        String::from_utf8_lossy(&validate_output.stderr)
    );
}

// ============================================================================
// Test 2: `astrbot run` starts and binds Dashboard port within 5 seconds
// ============================================================================
#[tokio::test]
async fn test_e2e_run_startup() {
    ensure_binary_built();

    let temp_dir = tempfile::tempdir().expect("create temp dir");
    let config_path = temp_dir.path().join("astrbot_config.yaml");
    let db_path = temp_dir.path().join("test.db");
    let dashboard_port = find_free_port();

    let config = minimal_config_yaml(dashboard_port, db_path.to_str().unwrap());
    fs::write(&config_path, config).expect("write test config");

    // Spawn `astrbot run` in the background
    let mut child = Command::new(astrbot_bin())
        .args([
            "run",
            "--config",
            config_path.to_str().unwrap(),
        ])
        .current_dir(temp_dir.path())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn astrbot run");

    // Poll the dashboard port for up to 5 seconds
    let started = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        async {
            loop {
                match tokio::net::TcpStream::connect(("127.0.0.1", dashboard_port)).await {
                    Ok(_) => return true,
                    Err(_) => {
                        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                    }
                }
            }
        },
    )
    .await
    .unwrap_or(false);

    // Always clean up the child process
    let _ = child.kill();
    let _ = child.wait();

    assert!(
        started,
        "Dashboard should bind to port {} within 5 seconds",
        dashboard_port
    );
}

// ============================================================================
// Test 3: `astrbot validate` returns 0 on valid config
// ============================================================================
#[test]
fn test_e2e_validate() {
    ensure_binary_built();

    let temp_dir = tempfile::tempdir().expect("create temp dir");
    let config_path = temp_dir.path().join("astrbot_config.yaml");
    let db_path = temp_dir.path().join("test.db");

    let config = minimal_config_yaml(find_free_port(), db_path.to_str().unwrap());
    fs::write(&config_path, config).expect("write test config");

    let output = Command::new(astrbot_bin())
        .args([
            "validate",
            "--config",
            config_path.to_str().unwrap(),
        ])
        .env("RUST_LOG", "info")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("spawn astrbot validate");

    assert!(
        output.status.success(),
        "astrbot validate should return 0 on a valid config. stdout: {}, stderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    // tracing_subscriber with fmt::init() writes to stdout by default
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Configuration is valid"),
        "validate should print success message to stdout. got stdout: {}",
        stdout
    );
}
