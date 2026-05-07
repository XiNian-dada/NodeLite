mod collector;

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use clap::Parser;
use tokio::fs;
use tracing::info;
use ximonitor_proto::{AgentConfig, parse_agent_config};

use crate::collector::new_collector;

#[derive(Debug, Parser)]
#[command(name = "ximonitor-agent")]
#[command(about = "XiMonitor Linux agent")]
struct Cli {
    #[arg(long, default_value = "config/agent.toml")]
    config: PathBuf,
    #[arg(long)]
    sample_once: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    init_tracing();

    let cli = Cli::parse();
    let config = load_agent_config(&cli.config).await?;
    let mut collector = new_collector();
    let identity = collector.collect_identity(&config, env!("CARGO_PKG_VERSION"))?;

    info!(
        node_id = %identity.node_id,
        node_label = %identity.node_label,
        "agent configuration loaded"
    );

    if cli.sample_once {
        let snapshot = collector.collect_snapshot()?;
        let output = serde_json::json!({
            "identity": identity,
            "snapshot": snapshot,
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&output).context("serialize sample output")?
        );
        return Ok(());
    }

    Err(anyhow!(
        "agent runtime not wired yet; run with --sample-once until the websocket loop lands"
    ))
}

async fn load_agent_config(path: &Path) -> Result<AgentConfig> {
    let content = fs::read_to_string(path)
        .await
        .with_context(|| format!("failed to read config file {}", path.display()))?;
    parse_agent_config(&content)
        .map_err(|error| anyhow!("failed to parse {}: {error}", path.display()))
}

fn init_tracing() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "ximonitor_agent=info".into()),
        )
        .with_target(false)
        .compact()
        .init();
}
