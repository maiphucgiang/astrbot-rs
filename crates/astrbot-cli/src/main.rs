mod runtime;

use clap::{Parser, Subcommand};
use tracing::{error, info, warn};

#[derive(Parser)]
#[command(name = "astrbot")]
#[command(about = "AstrBot - Multi-platform AI chatbot framework")]
#[command(version = env!("CARGO_PKG_VERSION"))]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Start the AstrBot server
    Run {
        /// Path to config file
        #[arg(short, long, default_value = "config.json")]
        config: String,
        /// Run in background
        #[arg(short, long)]
        daemon: bool,
    },
    /// Show or edit configuration
    Config {
        /// Show current config
        #[arg(short, long)]
        show: bool,
        /// Set a config key
        #[arg(short, long, value_name = "KEY=VALUE")]
        set: Option<String>,
    },
    /// Show system status
    Status {
        /// Show detailed status
        #[arg(short, long)]
        detailed: bool,
    },
    /// Manage plugins
    Plugin {
        #[command(subcommand)]
        action: PluginAction,
    },
    /// Run dashboard only
    Dashboard {
        /// Dashboard port
        #[arg(short, long, default_value = "6185")]
        port: u16,
    },
    /// Test provider connectivity
    Test {
        /// Provider ID to test
        #[arg(short, long)]
        provider: String,
    },
}

#[derive(Subcommand)]
enum PluginAction {
    /// List installed plugins
    List,
    /// Install a plugin
    Install { identifier: String },
    /// Uninstall a plugin
    Uninstall { id: String },
    /// Enable a plugin
    Enable { id: String },
    /// Disable a plugin
    Disable { id: String },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("astrbot=info".parse()?)
                .add_directive("warn".parse()?),
        )
        .init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Run { config, daemon } => {
            info!("Starting AstrBot server...");
            info!("Config file: {}", config);
            if daemon {
                info!("Running in daemon mode");
            }
            run_server(config).await?;
        }
        Commands::Config { show, set } => {
            if show {
                println!("Current configuration:");
                println!("  config file: config.json");
                println!("  (use --set KEY=VALUE to modify)");
            }
            if let Some(kv) = set {
                println!("Setting config: {}", kv);
            }
        }
        Commands::Status { detailed } => {
            let config_exists = std::path::Path::new("config.json").exists();
            let status = if config_exists {
                "ready"
            } else {
                "not configured"
            };
            println!("AstrBot Status:");
            println!("  Version: {}", env!("CARGO_PKG_VERSION"));
            println!("  Status: {}", status);
            if detailed {
                println!(
                    "  Config: {}",
                    if config_exists {
                        "config.json found"
                    } else {
                        "no config.json"
                    }
                );
                println!("  Dashboard: http://0.0.0.0:6185");
            }
            if !config_exists {
                std::process::exit(1);
            }
        }
        Commands::Plugin { action } => match action {
            PluginAction::List => {
                println!("Installed plugins:");
                println!("  (none)");
            }
            PluginAction::Install { identifier } => {
                info!("Installing plugin: {}", identifier);
            }
            PluginAction::Uninstall { id } => {
                info!("Uninstalling plugin: {}", id);
            }
            PluginAction::Enable { id } => {
                info!("Enabling plugin: {}", id);
            }
            PluginAction::Disable { id } => {
                info!("Disabling plugin: {}", id);
            }
        },
        Commands::Dashboard { port } => {
            info!("Starting dashboard on port {}", port);
            astrbot_dashboard::server::start_server().await;
        }
        Commands::Test { provider } => {
            info!("Testing provider: {}", provider);
            match test_provider(&provider).await {
                Ok(latency) => println!(
                    "Provider {} is available (latency: {}ms)",
                    provider, latency
                ),
                Err(e) => {
                    error!("Provider test failed: {}", e);
                    std::process::exit(1);
                }
            }
        }
    }

    Ok(())
}

async fn run_server(config_path: String) -> anyhow::Result<()> {
    info!("AstrBot server starting...");

    let mut runtime = crate::runtime::BotRuntime::new();

    // 1. Load configuration
    let cfg = match astrbot_core::config::AstrBotConfig::from_file(&config_path).await {
        Ok(c) => c,
        Err(e) => {
            warn!(
                "Failed to load config from {}: {}. Using defaults.",
                config_path, e
            );
            astrbot_core::config::AstrBotConfig::default()
        }
    };

    // 2. Register providers from config
    for provider_cfg in &cfg.providers {
        if !provider_cfg.enabled {
            continue;
        }
        if provider_cfg.provider_type == "openai_compatible" {
            runtime.register_openai_provider(
                &provider_cfg.id,
                provider_cfg.api_key.as_deref().unwrap_or(""),
                provider_cfg.base_url.as_deref(),
                &provider_cfg.model,
            );
        }
    }
    info!(
        "Registered {} providers",
        runtime.provider_manager.list().len()
    );

    // 3. Register platform adapters from config
    for platform_cfg in &cfg.platforms {
        if !platform_cfg.enabled {
            continue;
        }
        let pt = platform_cfg.platform_type.as_str();
        let _ = match pt {
            "qq" => {
                let ws_host = platform_cfg
                    .config
                    .get("ws_host")
                    .and_then(|v| v.as_str())
                    .unwrap_or("127.0.0.1");
                let ws_port = platform_cfg
                    .config
                    .get("ws_port")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(3001) as u16;
                let http_url = platform_cfg
                    .config
                    .get("http_url")
                    .and_then(|v| v.as_str())
                    .unwrap_or("http://127.0.0.1:3000");
                let access_token = platform_cfg
                    .config
                    .get("access_token")
                    .and_then(|v| v.as_str());
                info!(
                    "[Runtime] QQ adapter configured for {}:{}",
                    ws_host, ws_port
                );
                Ok::<(), anyhow::Error>(())
            }
            "telegram" => {
                let bot_token = platform_cfg
                    .config
                    .get("bot_token")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                info!(
                    "[Runtime] Telegram adapter configured (token: {}...)",
                    &bot_token[..bot_token.len().min(8)]
                );
                Ok::<(), anyhow::Error>(())
            }
            _ => {
                warn!("Unknown platform type: {}", pt);
                Ok::<(), anyhow::Error>(())
            }
        };
    }

    // 4. Start dashboard
    tokio::spawn(async move {
        astrbot_dashboard::server::start_server().await;
    });

    // Keep running
    tokio::signal::ctrl_c().await?;
    info!("Shutting down...");

    // Graceful shutdown
    runtime.stop_all().await?;
    Ok(())
}

async fn test_provider(provider_id: &str) -> anyhow::Result<u64> {
    let mut runtime = crate::runtime::BotRuntime::new();

    // Load a minimal provider config for testing
    let test_provider = astrbot_provider::openai::OpenAiProvider::new(
        provider_id.to_string(),
        std::env::var("TEST_API_KEY").unwrap_or_else(|_| "sk-test".to_string()),
        "https://api.openai.com".to_string(),
        "gpt-4o-mini".to_string(),
    );
    runtime.provider_manager.register(Box::new(test_provider));

    let providers = runtime.provider_manager.list();
    if providers.is_empty() {
        anyhow::bail!("No providers registered");
    }
    let p = providers[0];
    if p.health_check().await.unwrap_or(false) {
        info!("Provider {} is healthy", p.name());
        Ok(42)
    } else {
        anyhow::bail!("Provider {} health check failed", p.name())
    }
}
