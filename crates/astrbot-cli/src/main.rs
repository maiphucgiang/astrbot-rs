mod runtime;

use clap::{Parser, Subcommand};
use std::path::Path;
use std::sync::Arc;
use tracing::{error, info, warn};

#[derive(Parser)]
#[command(name = "astrbot")]
#[command(about = "AstrBot — Multi-platform AI chatbot framework (Rust rewrite)")]
#[command(version = env!("CARGO_PKG_VERSION"))]
#[command(disable_help_subcommand = false)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Init {
        #[arg(short, long, default_value = ".")]
        dir: String,
        #[arg(short, long)]
        minimal: bool,
    },
    Run {
        #[arg(short, long, default_value = "config.json")]
        config: String,
        #[arg(short, long)]
        daemon: bool,
    },
    Config {
        #[arg(short, long)]
        show: bool,
        #[arg(short, long, value_name = "KEY=VALUE")]
        set: Option<String>,
        #[arg(short, long, default_value = "config.json")]
        file: String,
    },
    Status {
        #[arg(short, long)]
        detailed: bool,
        #[arg(short, long, default_value = "config.json")]
        config: String,
    },
    Plugin {
        #[command(subcommand)]
        action: PluginAction,
    },
    Validate {
        #[arg(short, long, default_value = "config.json")]
        config: String,
    },
    Dashboard {
        #[arg(short, long, default_value = "6185")]
        port: u16,
        #[arg(short, long, default_value = "config.json")]
        config: String,
    },
    Test {
        #[arg(short, long)]
        provider: String,
        #[arg(short, long)]
        api_key: Option<String>,
    },
}

#[derive(Subcommand)]
enum PluginAction {
    List,
    Install {
        identifier: String,
    },
    Uninstall {
        id: String,
    },
    Enable {
        id: String,
    },
    Disable {
        id: String,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("astrbot=info".parse()?)
                .add_directive("warn".parse()?),
        )
        .init();
    let cli = Cli::parse();
    match cli.command {
        Commands::Init { dir, minimal } => cmd_init(&dir, minimal).await?,
        Commands::Run { config, daemon } => cmd_run(config, daemon).await?,
        Commands::Config { show, set, file } => cmd_config(show, set, &file).await?,
        Commands::Status { detailed, config } => cmd_status(detailed, &config).await?,
        Commands::Plugin { action } => cmd_plugin(action).await?,
        Commands::Validate { config } => cmd_validate(&config).await?,
        Commands::Dashboard { port, config } => cmd_dashboard(port, &config).await?,
        Commands::Test { provider, api_key } => cmd_test(&provider, api_key.as_deref()).await?,
    }
    Ok(())
}

async fn cmd_init(dir: &str, minimal: bool) -> anyhow::Result<()> {
    let base = Path::new(dir);
    std::fs::create_dir_all(base)?;
    for sub in &["data", "plugins", "logs", "tmp"] {
        std::fs::create_dir_all(base.join(sub))?;
    }
    let config_path = base.join("config.json");
    if config_path.exists() {
        warn!("config.json already exists at {:?}", config_path);
        println!("⚠️ config.json already exists — skipping generation");
    } else {
        let cfg = if minimal {
            minimal_config()
        } else {
            default_config()
        };
        let json = serde_json::to_string_pretty(&cfg)?;
        tokio::fs::write(&config_path, json).await?;
        info!("Generated config.json at {:?}", config_path);
    }
    let env_path = base.join(".env");
    if !env_path.exists() {
        tokio::fs::write(
            &env_path,
            "# AstrBot Environment Variables\n# OPENAI_API_KEY=sk-...\n# TELEGRAM_BOT_TOKEN=...\n",
        )
        .await?;
    }
    println!(
        "✅ AstrBot workspace initialized at {}",
        base.canonicalize()?.display()
    );
    println!(" Config: {}", config_path.display());
    println!(" Directories: data/ plugins/ logs/ tmp/");
    Ok(())
}

fn default_config() -> serde_json::Value {
    serde_json::json!({
        "nickname": "AstrBot", "prefixes": ["/"], "admins": [],
        "platforms": [], "providers": [], "plugins": {},
        "webui": { "enabled": true, "host": "0.0.0.0", "port": 6185, "jwt_secret": null, "tls_cert": null, "tls_key": null },
        "log_level": "info", "database_url": "sqlite:data/data.db"
    })
}

fn minimal_config() -> serde_json::Value {
    serde_json::json!({
        "nickname": "AstrBot", "prefixes": ["/"], "admins": [],
        "platforms": [], "providers": [], "plugins": {},
        "webui": { "enabled": true, "host": "0.0.0.0", "port": 6185 },
        "log_level": "info", "database_url": "sqlite:data/data.db"
    })
}

async fn cmd_run(config_path: String, daemon: bool) -> anyhow::Result<()> {
    info!("Starting AstrBot server...");
    info!("Config file: {}", config_path);
    if daemon {
        info!("Running in daemon mode (not yet implemented)");
    }
    let mut runtime = crate::runtime::BotRuntime::new(std::path::PathBuf::from("plugins"));
    let cfg = match astrbot_core::config::AstrBotConfig::from_file(&config_path).await {
        Ok(c) => {
            info!("Loaded config from {}", config_path);
            c
        }
        Err(e) => {
            warn!(
                "Failed to load config from {}: {}. Using defaults.",
                config_path, e
            );
            astrbot_core::config::AstrBotConfig::default()
        }
    };
    for provider_cfg in &cfg.providers {
        if !provider_cfg.enabled {
            continue;
        }
        if provider_cfg.provider_type == "openai_compatible" || provider_cfg.provider_type == "openai" {
            runtime.register_openai_provider(
                &provider_cfg.id,
                provider_cfg.api_key.as_deref().unwrap_or(""),
                provider_cfg.base_url.as_deref(),
                &provider_cfg.model,
            );
        }
    }
    info!("Registered {} providers", runtime.provider_manager.list().len());
    runtime.start().await?;
    for platform_cfg in &cfg.platforms {
        if !platform_cfg.enabled {
            continue;
        }
        info!(
            "Platform configured: {} (type={})",
            platform_cfg.id, platform_cfg.platform_type
        );
    }
    let pipeline = runtime.pipeline.clone();
    tokio::spawn(async move {
        astrbot_dashboard::server::start_server(pipeline).await;
    });
    tokio::signal::ctrl_c().await?;
    info!("Shutting down...");
    runtime.stop_all().await?;
    Ok(())
}

async fn cmd_config(
    show: bool,
    set: Option<String>,
    file: &str,
) -> anyhow::Result<()> {
    if show {
        let path = Path::new(file);
        if !path.exists() {
            println!("❌ Config file not found: {}", file);
            return Err(anyhow::anyhow!("config file not found"));
        }
        let content = tokio::fs::read_to_string(path).await?;
        println!("{}", content);
        return Ok(());
    }
    if let Some(kv) = set {
        let parts: Vec<&str> = kv.splitn(2, '=').collect();
        if parts.len() != 2 {
            println!(
                "❌ Invalid format: expected KEY=VALUE, got {}",
                kv
            );
            return Err(anyhow::anyhow!("invalid KEY=VALUE format"));
        }
        println!("Setting config: {} = {}", parts[0], parts[1]);
        println!(" (config mutation not yet implemented)");
        return Ok(());
    }
    let path = Path::new(file);
    let exists = path.exists();
    println!("AstrBot Configuration");
    println!(" File: {}", file);
    println!(
        " Exists: {}",
        if exists { "✅ yes" } else { "❌ no" }
    );
    if exists {
        let content = tokio::fs::read_to_string(path).await?;
        let cfg: serde_json::Value = serde_json::from_str(&content)?;
        println!(
            " Nickname: {}",
            cfg["nickname"].as_str().unwrap_or("AstrBot")
        );
        println!(
            " Platforms: {}",
            cfg["platforms"].as_array().map(|a| a.len()).unwrap_or(0)
        );
        println!(
            " Providers: {}",
            cfg["providers"].as_array().map(|a| a.len()).unwrap_or(0)
        );
    }
    Ok(())
}

async fn cmd_status(detailed: bool, config_file: &str) -> anyhow::Result<()> {
    let config_exists = Path::new(config_file).exists();
    let data_dir_exists = Path::new("data").exists();
    let plugins_dir_exists = Path::new("plugins").exists();
    println!("AstrBot Status");
    println!("==============");
    println!(" Version: {}", env!("CARGO_PKG_VERSION"));
    println!(
        " Config: {} {}",
        config_file,
        if config_exists { "✅" } else { "❌" }
    );
    println!(
        " Data dir: {} {}",
        "data/",
        if data_dir_exists { "✅" } else { "❌" }
    );
    println!(
        " Plugins: {} {}",
        "plugins/",
        if plugins_dir_exists { "✅" } else { "❌" }
    );
    if detailed && config_exists {
        let content = tokio::fs::read_to_string(config_file).await?;
        let cfg: serde_json::Value = serde_json::from_str(&content)?;
        println!("\nDetailed:");
        println!(
            " Nickname: {}",
            cfg["nickname"].as_str().unwrap_or("AstrBot")
        );
        println!(
            " Log level: {}",
            cfg["log_level"].as_str().unwrap_or("info")
        );
        println!(
            " Database: {}",
            cfg["database_url"]
                .as_str()
                .unwrap_or("sqlite:data/data.db")
        );
        println!(
            " Platforms: {}",
            cfg["platforms"].as_array().map(|a| a.len()).unwrap_or(0)
        );
        println!(
            " Providers: {}",
            cfg["providers"].as_array().map(|a| a.len()).unwrap_or(0)
        );
        println!(
            " Dashboard: http://{}:{}",
            cfg["webui"]["host"].as_str().unwrap_or("0.0.0.0"),
            cfg["webui"]["port"].as_u64().unwrap_or(6185)
        );
    }
    if !config_exists {
        return Err(anyhow::anyhow!("config file not found"));
    }
    Ok(())
}

async fn cmd_plugin(action: PluginAction) -> anyhow::Result<()> {
    match action {
        PluginAction::List => {
            println!("Installed plugins:");
            println!(" (none — registry not yet wired)");
        }
        PluginAction::Install { identifier } => {
            info!("Installing plugin: {}", identifier);
            println!(
                "📦 Installing {}... (installer not yet implemented)",
                identifier
            );
        }
        PluginAction::Uninstall { id } => {
            info!("Uninstalling plugin: {}", id);
            println!("🗑️ Uninstalling {}... (installer not yet implemented)", id);
        }
        PluginAction::Enable { id } => {
            info!("Enabling plugin: {}", id);
            println!("▶️ Enabling {}... (state manager not yet implemented)", id);
        }
        PluginAction::Disable { id } => {
            info!("Disabling plugin: {}", id);
            println!("⏸️ Disabling {}... (state manager not yet implemented)", id);
        }
    }
    Ok(())
}

async fn cmd_validate(config_path: &str) -> anyhow::Result<()> {
    let path = Path::new(config_path);
    if !path.exists() {
        error!("Config file not found: {}", config_path);
        println!("❌ Config file not found: {}", config_path);
        return Err(anyhow::anyhow!("config file not found"));
    }
    let content = tokio::fs::read_to_string(path).await?;
    let cfg: serde_json::Value = serde_json::from_str(&content)?;
    let mut errors = Vec::new();
    if cfg["nickname"].as_str().is_none() {
        errors.push("missing or invalid 'nickname'");
    }
    if !cfg["database_url"].as_str().is_some() {
        errors.push("missing 'database_url'");
    }
    if errors.is_empty() {
        println!("✅ Configuration is valid: {}", config_path);
        Ok(())
    } else {
        println!("❌ Configuration errors:");
        for e in &errors {
            println!(" - {}", e);
        }
        Err(anyhow::anyhow!("config validation failed"))
    }
}

async fn cmd_dashboard(port: u16, _config: &str) -> anyhow::Result<()> {
    info!("Starting dashboard on port {}", port);
    println!("🚀 Dashboard starting on http://0.0.0.0:{}", port);
    astrbot_dashboard::server::start_server(None).await;
    Ok(())
}

async fn cmd_test(provider_id: &str, api_key: Option<&str>) -> anyhow::Result<()> {
    info!("Testing provider: {}", provider_id);
    let mut runtime = crate::runtime::BotRuntime::new(std::path::PathBuf::from("plugins"));
    let key = api_key
        .map(|s| s.to_string())
        .unwrap_or_else(|| {
            std::env::var("TEST_API_KEY").unwrap_or_else(|_| "sk-test".to_string())
        });
    let test_provider = astrbot_provider::openai::OpenAiProvider::new(
        provider_id.to_string(),
        key,
        "https://api.openai.com".to_string(),
        "gpt-4o-mini".to_string(),
    );
    runtime.provider_manager.register(Arc::new(test_provider));
    let providers = runtime.provider_manager.list();
    if providers.is_empty() {
        anyhow::bail!("No providers registered");
    }
    let p = providers[0];
    if p.health_check().await.unwrap_or(false) {
        println!("✅ Provider {} is healthy", p.name());
        Ok(())
    } else {
        anyhow::bail!("Provider {} health check failed", p.name())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_cmd_init() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().to_str().unwrap();
        cmd_init(dir, false).await.unwrap();
        assert!(temp.path().join("config.json").exists());
        assert!(temp.path().join("data").exists());
        assert!(temp.path().join("plugins").exists());
        assert!(temp.path().join("logs").exists());
        assert!(temp.path().join("tmp").exists());
        assert!(temp.path().join(".env").exists());
        let content = tokio::fs::read_to_string(temp.path().join("config.json"))
            .await
            .unwrap();
        let cfg: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert_eq!(cfg["nickname"].as_str().unwrap(), "AstrBot");
        assert_eq!(cfg["webui"]["port"].as_u64().unwrap(), 6185);
    }

    #[tokio::test]
    async fn test_cmd_init_minimal() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().to_str().unwrap();
        cmd_init(dir, true).await.unwrap();
        let content = tokio::fs::read_to_string(temp.path().join("config.json"))
            .await
            .unwrap();
        let cfg: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert!(cfg["webui"]["tls_cert"].is_null());
    }

    #[tokio::test]
    async fn test_cmd_validate_ok() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("config.json");
        let cfg = default_config();
        tokio::fs::write(&path, serde_json::to_string_pretty(&cfg).unwrap())
            .await
            .unwrap();
        cmd_validate(path.to_str().unwrap()).await.unwrap();
    }

    #[tokio::test]
    async fn test_cmd_validate_fail() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("bad.json");
        tokio::fs::write(&path, r#"{"invalid": true}"#)
            .await
            .unwrap();
        let result = cmd_validate(path.to_str().unwrap()).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_cmd_config_show() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("config.json");
        let cfg = default_config();
        tokio::fs::write(&path, serde_json::to_string_pretty(&cfg).unwrap())
            .await
            .unwrap();
        cmd_config(true, None, path.to_str().unwrap()).await.unwrap();
    }

    #[tokio::test]
    async fn test_cmd_config_set_invalid() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("config.json");
        let cfg = default_config();
        tokio::fs::write(&path, serde_json::to_string_pretty(&cfg).unwrap())
            .await
            .unwrap();
        let result = cmd_config(false, Some("badformat".to_string()), path.to_str().unwrap()).await;
        assert!(result.is_ok() || result.is_err());
    }

    #[tokio::test]
    async fn test_cmd_status_present() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("config.json");
        let cfg = default_config();
        tokio::fs::write(&path, serde_json::to_string_pretty(&cfg).unwrap())
            .await
            .unwrap();
        cmd_status(false, path.to_str().unwrap()).await.unwrap();
    }

    #[tokio::test]
    async fn test_cmd_plugin_list() {
        cmd_plugin(PluginAction::List).await.unwrap();
    }

    #[test]
    fn test_default_config_structure() {
        let cfg = default_config();
        assert_eq!(cfg["nickname"].as_str().unwrap(), "AstrBot");
        assert!(cfg["platforms"].is_array());
        assert!(cfg["providers"].is_array());
        assert!(cfg["webui"]["port"].is_u64());
    }

    #[test]
    fn test_minimal_vs_default() {
        let default = serde_json::to_string(&default_config()).unwrap();
        let minimal = serde_json::to_string(&minimal_config()).unwrap();
        assert!(minimal.len() < default.len());
    }
}
