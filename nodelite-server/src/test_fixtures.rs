//! Shared fixture data and handshake helpers for tests and the opt-in benchmark adapter.

use std::time::Duration;

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};
use futures::{SinkExt, StreamExt};
use nodelite_proto::{
    DiskUsage, LoadAverage, MemoryUsage, NetworkCounters, NodeIdentity, NodeSnapshot, NoticeLevel,
    WireMessage,
};
use tokio::net::TcpStream;
use tokio::time::timeout;
use tokio_tungstenite::tungstenite::Message;

#[path = "test_support/config.rs"]
mod config;
pub use config::{test_server_config, test_ws_config};

type TestSocket = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<TcpStream>>;

pub fn synthetic_identity(
    node_id: &str,
    node_label: &str,
    agent_version: &str,
    kernel_version: Option<&str>,
    tag: &str,
) -> NodeIdentity {
    NodeIdentity {
        node_id: node_id.to_string(),
        node_label: node_label.to_string(),
        hostname: format!("{node_id}.example.internal"),
        os: "Linux".to_string(),
        kernel_version: kernel_version.map(str::to_string),
        cpu_model: Some("Rust Hypervisor".to_string()),
        cpu_cores: 4,
        agent_version: agent_version.to_string(),
        boot_time: Some(Utc::now()),
        tags: vec![tag.to_string()],
    }
}

pub fn fake_snapshot_at(uptime_secs: u64, collected_at: DateTime<Utc>) -> NodeSnapshot {
    NodeSnapshot {
        collected_at,
        cpu_usage_percent: Some(12.5 + (uptime_secs % 7) as f64),
        load: LoadAverage {
            one: 0.3,
            five: 0.4,
            fifteen: 0.5,
        },
        memory: MemoryUsage {
            total_bytes: 4 * 1024 * 1024 * 1024,
            used_bytes: 1536 * 1024 * 1024,
            available_bytes: 2560 * 1024 * 1024,
            swap_total_bytes: 1024 * 1024 * 1024,
            swap_used_bytes: 64 * 1024 * 1024,
        },
        uptime_secs,
        disks: vec![DiskUsage {
            device: "/dev/vda".to_string(),
            mount_point: "/".to_string(),
            fs_type: "ext4".to_string(),
            total_bytes: 80 * 1024 * 1024 * 1024,
            available_bytes: 40 * 1024 * 1024 * 1024,
            used_bytes: 40 * 1024 * 1024 * 1024,
            used_percent: 50.0,
        }],
        network: NetworkCounters {
            total_rx_bytes: 512 * 1024 * uptime_secs,
            total_tx_bytes: 256 * 1024 * uptime_secs,
            rx_bytes_per_sec: Some(32_768.0 + uptime_secs as f64),
            tx_bytes_per_sec: Some(16_384.0 + uptime_secs as f64),
            packet_loss_percent: Some((uptime_secs % 3) as f64 * 0.1),
        },
    }
}

pub async fn send_wire_message(socket: &mut TestSocket, message: &WireMessage) -> Result<()> {
    let payload = serde_json::to_string(message).context("serialize wire message")?;
    socket
        .send(Message::Text(payload.into()))
        .await
        .context("send websocket message")
}

pub async fn wait_for_authenticated_notice(
    socket: &mut TestSocket,
    node_id: &str,
    timeout_duration: Duration,
) -> Result<()> {
    timeout(timeout_duration, async {
        loop {
            let Some(frame) = socket.next().await else {
                bail!("socket closed before authenticated notice");
            };
            match frame.context("receive websocket frame")? {
                Message::Text(text) => {
                    let message: WireMessage =
                        serde_json::from_str(&text).context("decode wire message")?;
                    match message {
                        WireMessage::ServerNotice(notice) if notice.message == "authenticated" => {
                            return Ok(());
                        }
                        WireMessage::Ping(ping) => {
                            send_wire_message(
                                socket,
                                &WireMessage::Pong(nodelite_proto::PongMessage {
                                    nonce: ping.nonce,
                                }),
                            )
                            .await?;
                        }
                        WireMessage::ServerNotice(notice) if notice.level == NoticeLevel::Error => {
                            bail!("server rejected {node_id}: {}", notice.message);
                        }
                        _ => {}
                    }
                }
                Message::Ping(payload) => {
                    socket
                        .send(Message::Pong(payload))
                        .await
                        .context("reply websocket ping")?;
                }
                Message::Close(frame) => {
                    bail!("socket closed before auth: {frame:?}");
                }
                _ => {}
            }
        }
    })
    .await
    .context("timed out waiting for authenticated notice")?
}
