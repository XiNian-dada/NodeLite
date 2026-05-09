mod collector;

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use clap::Parser;
use futures::{SinkExt, StreamExt};
use tokio::fs;
use tokio::time::interval;
use tokio::time::sleep;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;
use tracing::{info, warn};
use url::Url;
use ximonitor_proto::{
    AgentConfig, HelloMessage, MetricsMessage, NoticeLevel, PingMessage, PongMessage,
    ServerNoticeMessage, WireMessage, parse_agent_config,
};

use crate::collector::new_collector;

const INSECURE_TRANSPORT_WARN_INTERVAL_SECS: u64 = 900;

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
    install_rustls_crypto_provider()?;

    let cli = Cli::parse();
    let config = load_agent_config(&cli.config).await?;
    let mut collector = new_collector();
    let identity = collector.collect_identity(&config, agent_build_version())?;

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

    spawn_insecure_transport_warning(config.server.clone());
    run_forever(config, collector, identity).await
}

fn install_rustls_crypto_provider() -> Result<()> {
    rustls::crypto::ring::default_provider()
        .install_default()
        .map_err(|_| anyhow!("failed to install rustls crypto provider"))
}

fn agent_build_version() -> &'static str {
    option_env!("XIMONITOR_BUILD_VERSION").unwrap_or(env!("CARGO_PKG_VERSION"))
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

async fn run_forever(
    config: AgentConfig,
    mut collector: crate::collector::HostCollector,
    identity: ximonitor_proto::NodeIdentity,
) -> Result<()> {
    let mut attempt = 0_u32;

    loop {
        match run_session(&config, &mut collector, &identity).await {
            Ok(()) => {
                attempt = 0;
            }
            Err(error) => {
                let delay = reconnect_delay(attempt);
                warn!(
                    server = %config.server,
                    delay_secs = delay.as_secs(),
                    error = ?error,
                    "agent session ended; retrying after backoff"
                );
                sleep(delay).await;
                attempt = attempt.saturating_add(1);
            }
        }
    }
}

async fn run_session(
    config: &AgentConfig,
    collector: &mut crate::collector::HostCollector,
    identity: &ximonitor_proto::NodeIdentity,
) -> Result<()> {
    let (socket, _) = connect_async(config.server.as_str())
        .await
        .with_context(|| format!("failed to connect to {}", config.server))?;
    let (mut sender, mut receiver) = socket.split();

    send_wire_message(
        &mut sender,
        &WireMessage::Hello(HelloMessage {
            token: config.token.clone(),
            identity: identity.clone(),
        }),
    )
    .await?;

    send_metrics(&mut sender, collector).await?;

    let mut report_ticker = interval(Duration::from_secs(config.report_interval_secs));

    loop {
        tokio::select! {
            _ = report_ticker.tick() => {
                send_metrics(&mut sender, collector).await?;
            }
            incoming = receiver.next() => {
                let Some(frame) = incoming else {
                    return Err(anyhow!("server closed websocket connection"));
                };
                let frame = frame.context("failed to read websocket frame")?;
                match frame {
                    Message::Text(text) => {
                        match serde_json::from_str::<WireMessage>(&text).context("invalid websocket json")? {
                            WireMessage::Ping(PingMessage { nonce }) => {
                                send_wire_message(&mut sender, &WireMessage::Pong(PongMessage { nonce })).await?;
                            }
                            WireMessage::ServerNotice(ServerNoticeMessage { level, message }) => {
                                log_notice(level, &message);
                            }
                            WireMessage::Hello(_) | WireMessage::Metrics(_) | WireMessage::Pong(_) => {
                                return Err(anyhow!("received unexpected websocket message from server"));
                            }
                        }
                    }
                    Message::Ping(payload) => {
                        sender.send(Message::Pong(payload)).await.context("failed to reply to ping frame")?;
                    }
                    Message::Pong(_) => {}
                    Message::Close(frame) => {
                        return Err(anyhow!("server closed websocket connection: {:?}", frame));
                    }
                    Message::Binary(_) | Message::Frame(_) => {
                        return Err(anyhow!("binary websocket frames are not supported"));
                    }
                }
            }
        }
    }
}

async fn send_metrics(
    sender: &mut futures::stream::SplitSink<
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
        Message,
    >,
    collector: &mut crate::collector::HostCollector,
) -> Result<()> {
    let snapshot = collector.collect_snapshot()?;
    send_wire_message(sender, &WireMessage::Metrics(MetricsMessage { snapshot })).await
}

async fn send_wire_message(
    sender: &mut futures::stream::SplitSink<
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
        Message,
    >,
    message: &WireMessage,
) -> Result<()> {
    let payload = serde_json::to_string(message).context("serialize websocket message")?;
    sender
        .send(Message::Text(payload.into()))
        .await
        .context("send websocket message")?;
    Ok(())
}

fn log_notice(level: NoticeLevel, message: &str) {
    match level {
        NoticeLevel::Info => info!(message = %message, "server notice"),
        NoticeLevel::Warn => tracing::warn!(message = %message, "server notice"),
        NoticeLevel::Error => tracing::error!(message = %message, "server notice"),
    }
}

fn reconnect_delay(attempt: u32) -> Duration {
    let seconds = match attempt {
        0 => 1,
        1 => 2,
        2 => 4,
        3 => 8,
        4 => 16,
        5 => 32,
        _ => 60,
    };
    Duration::from_secs(seconds)
}

fn spawn_insecure_transport_warning(server_url: String) {
    if !uses_insecure_remote_transport(&server_url) {
        return;
    }

    tokio::spawn(async move {
        let mut ticker = interval(Duration::from_secs(INSECURE_TRANSPORT_WARN_INTERVAL_SECS));
        loop {
            ticker.tick().await;
            warn!(
                server = %server_url,
                "agent is configured without TLS; use a wss:// server URL in production",
            );
        }
    });
}

fn uses_insecure_remote_transport(server_url: &str) -> bool {
    let Ok(url) = Url::parse(server_url) else {
        return false;
    };
    if url.scheme() != "ws" {
        return false;
    }

    !host_is_local(url.host_str())
}

fn host_is_local(host: Option<&str>) -> bool {
    let Some(host) = host else {
        return false;
    };
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }

    host.parse::<std::net::IpAddr>()
        .map(|ip| ip.is_loopback())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::uses_insecure_remote_transport;

    #[test]
    fn warns_for_remote_ws_transport() {
        assert!(uses_insecure_remote_transport(
            "ws://monitor.example.com/ws"
        ));
        assert!(uses_insecure_remote_transport("ws://203.0.113.10/ws"));
    }

    #[test]
    fn ignores_local_or_tls_agent_transport() {
        assert!(!uses_insecure_remote_transport(
            "wss://monitor.example.com/ws"
        ));
        assert!(!uses_insecure_remote_transport("ws://127.0.0.1:8080/ws"));
        assert!(!uses_insecure_remote_transport("ws://localhost:8080/ws"));
    }
}
