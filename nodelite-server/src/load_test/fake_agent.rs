use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use chrono::{DateTime, Utc};
use futures::SinkExt;
use tokio::sync::{Barrier, mpsc, watch};
use tokio::time::sleep;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;

use super::{AgentCredential, AgentWorkload, LOAD_TEST_TIMEOUT_SECS, TestSocket};
use crate::history::HistoryStore;
use crate::state::SharedState;
use crate::test_support::{fake_snapshot_at, synthetic_identity, wait_for_authenticated_notice};
use nodelite_proto::{
    DEFAULT_HISTORY_WRITE_INTERVAL_SECS, HelloMessage, MetricsMessage, NodeIdentity, NodeSnapshot,
    NodeStatus, WireMessage,
};

#[derive(Debug, Clone, Copy)]
pub(super) struct SeededHistoryRange {
    pub(super) start_at: DateTime<Utc>,
    pub(super) end_at: DateTime<Utc>,
}

pub(super) async fn run_fake_agent(
    addr: SocketAddr,
    credential: AgentCredential,
    workload: AgentWorkload,
    ready_tx: mpsc::UnboundedSender<String>,
    burst_barrier: Arc<Barrier>,
    mut stop_rx: watch::Receiver<bool>,
) -> Result<()> {
    let mut socket = connect_authenticated_fake_agent(addr, &credential, ready_tx).await?;
    send_metrics_workload(&mut socket, workload, burst_barrier).await?;

    let _ = stop_rx.changed().await;
    let _ = socket.close(None).await;
    Ok(())
}

pub(super) async fn run_fake_agent_session(
    addr: SocketAddr,
    credential: AgentCredential,
    workload: AgentWorkload,
    ready_tx: mpsc::UnboundedSender<String>,
    burst_barrier: Arc<Barrier>,
) -> Result<()> {
    let mut socket = connect_authenticated_fake_agent(addr, &credential, ready_tx).await?;
    send_metrics_workload(&mut socket, workload, burst_barrier).await?;
    if !workload.hold_after_send.is_zero() {
        sleep(workload.hold_after_send).await;
    }
    let _ = socket.close(None).await;
    Ok(())
}

pub(super) fn fake_identity(credential: &AgentCredential) -> NodeIdentity {
    synthetic_identity(
        &credential.node_id,
        &credential.node_label,
        "load-test",
        Some("6.8.0-load-test"),
        "load-test",
    )
}

pub(super) fn fake_snapshot(uptime_secs: u64) -> NodeSnapshot {
    fake_snapshot_at(uptime_secs, Utc::now())
}

pub(super) fn fake_snapshot_with_disks(uptime_secs: u64, disk_entries: usize) -> NodeSnapshot {
    let mut snapshot = fake_snapshot(uptime_secs);
    if disk_entries <= snapshot.disks.len() {
        return snapshot;
    }

    let template = snapshot.disks.first().cloned();
    if let Some(template) = template {
        snapshot.disks = (0..disk_entries)
            .map(|index| {
                let mut disk = template.clone();
                disk.device = format!("/dev/vd{}", (b'a' + (index % 26) as u8) as char);
                disk.mount_point = format!("/mnt/load-{index:02}");
                disk.used_percent = 35.0 + (index % 50) as f64;
                disk
            })
            .collect();
    }
    snapshot
}

pub(super) async fn wait_for_final_snapshots(
    shared: SharedState,
    credentials: &[AgentCredential],
    expected_uptime: u64,
    timeout_duration: Duration,
    require_online: bool,
) -> Result<()> {
    let started = Instant::now();
    let expected_nodes: HashSet<_> = credentials
        .iter()
        .map(|item| item.node_id.as_str())
        .collect();

    loop {
        let statuses = shared.list_statuses().await;
        let by_id: HashMap<_, _> = statuses
            .iter()
            .map(|status| (status.identity.node_id.as_str(), status))
            .collect();
        let all_ready = expected_nodes.iter().all(|node_id| {
            by_id.get(node_id).is_some_and(|status| {
                (!require_online || status.online)
                    && status
                        .snapshot
                        .as_ref()
                        .is_some_and(|snapshot| snapshot.uptime_secs == expected_uptime)
            })
        });
        if all_ready {
            return Ok(());
        }
        if started.elapsed() > timeout_duration {
            let mut unfinished = Vec::new();
            for node_id in &expected_nodes {
                match by_id.get(node_id) {
                    Some(status) => unfinished.push(format!(
                        "{} online={} uptime={:?}",
                        node_id,
                        status.online,
                        status
                            .snapshot
                            .as_ref()
                            .map(|snapshot| snapshot.uptime_secs)
                    )),
                    None => unfinished.push(format!("{node_id} missing")),
                }
            }
            bail!(
                "timed out waiting for final snapshots: {}",
                unfinished.join(", ")
            );
        }
        sleep(Duration::from_millis(20)).await;
    }
}

pub(super) async fn wait_for_all_offline(
    shared: SharedState,
    credentials: &[AgentCredential],
    timeout_duration: Duration,
) -> Result<()> {
    let started = Instant::now();
    let expected_nodes: HashSet<_> = credentials
        .iter()
        .map(|item| item.node_id.as_str())
        .collect();

    loop {
        let statuses = shared.list_statuses().await;
        let by_id: HashMap<_, _> = statuses
            .iter()
            .map(|status| (status.identity.node_id.as_str(), status))
            .collect();
        let all_offline = expected_nodes
            .iter()
            .all(|node_id| by_id.get(node_id).is_some_and(|status| !status.online));
        if all_offline {
            return Ok(());
        }
        if started.elapsed() > timeout_duration {
            let mut unfinished = Vec::new();
            for node_id in &expected_nodes {
                match by_id.get(node_id) {
                    Some(status) => unfinished.push(format!("{node_id} online={}", status.online)),
                    None => unfinished.push(format!("{node_id} missing")),
                }
            }
            bail!(
                "timed out waiting for all nodes to disconnect: {}",
                unfinished.join(", ")
            );
        }
        sleep(Duration::from_millis(20)).await;
    }
}

pub(super) async fn seed_history_points(
    history: HistoryStore,
    credential: &AgentCredential,
    points: usize,
) -> Result<SeededHistoryRange> {
    let now = Utc::now();
    let spacing_secs = nodelite_proto::DEFAULT_HISTORY_WRITE_INTERVAL_SECS as i64;
    let first_point_at = now - chrono::Duration::seconds((points as i64 - 1).max(0) * spacing_secs);
    let mut last_point_at = first_point_at;
    for index in 0..points {
        let recorded_at = first_point_at + chrono::Duration::seconds(index as i64 * spacing_secs);
        last_point_at = recorded_at;
        let status = NodeStatus {
            identity: fake_identity(credential),
            remote_ip: Some("127.0.0.1".to_string()),
            geoip_country: None,
            geoip_city: None,
            geoip_latitude: None,
            geoip_longitude: None,
            snapshot: Some(fake_snapshot_at(index as u64 + 1, recorded_at)),
            last_seen: Some(recorded_at),
            latency_ms: Some(6 + (index as u64 % 17)),
            online: true,
        };
        history.record_status(&status).await;
    }
    Ok(SeededHistoryRange {
        start_at: first_point_at,
        end_at: last_point_at,
    })
}

pub(super) async fn wait_for_seeded_history_points(
    history: HistoryStore,
    node_id: &str,
    range: SeededHistoryRange,
    expected_points: usize,
) -> Result<()> {
    let started = Instant::now();
    let max_points = ((range.end_at.timestamp() - range.start_at.timestamp()).max(1) as usize)
        + DEFAULT_HISTORY_WRITE_INTERVAL_SECS as usize;

    loop {
        let points = history
            .query_history_range(node_id, range.start_at, range.end_at, max_points)
            .await
            .with_context(|| format!("query seeded history for {node_id}"))?;
        if points.len() >= expected_points {
            return Ok(());
        }
        if started.elapsed() > Duration::from_secs(LOAD_TEST_TIMEOUT_SECS) {
            bail!(
                "timed out waiting for seeded history points for {node_id}: got {} / {}",
                points.len(),
                expected_points
            );
        }
        sleep(Duration::from_millis(20)).await;
    }
}

async fn connect_authenticated_fake_agent(
    addr: SocketAddr,
    credential: &AgentCredential,
    ready_tx: mpsc::UnboundedSender<String>,
) -> Result<TestSocket> {
    let url = format!("ws://{addr}/ws");
    let (mut socket, _response) = connect_async(url)
        .await
        .with_context(|| format!("connect fake agent {}", credential.node_id))?;

    let hello = WireMessage::Hello(HelloMessage {
        protocol_version: nodelite_proto::WIRE_PROTOCOL_VERSION,
        token: credential.token.clone(),
        identity: fake_identity(credential),
    });
    send_wire_message(&mut socket, &hello).await?;
    wait_for_authenticated_notice(
        &mut socket,
        &credential.node_id,
        Duration::from_secs(LOAD_TEST_TIMEOUT_SECS),
    )
    .await?;
    ready_tx
        .send(credential.node_id.clone())
        .map_err(|_| anyhow!("ready channel closed"))?;
    Ok(socket)
}

async fn send_metrics_workload(
    socket: &mut TestSocket,
    workload: AgentWorkload,
    burst_barrier: Arc<Barrier>,
) -> Result<()> {
    burst_barrier.wait().await;
    for step in 0..workload.metrics_per_node {
        let metrics = WireMessage::Metrics(MetricsMessage {
            snapshot: fake_snapshot_with_disks(workload.uptime_start + step, workload.disk_entries),
        });
        send_wire_message(socket, &metrics).await?;
        if !workload.inter_message_delay.is_zero() {
            sleep(workload.inter_message_delay).await;
        }
    }
    Ok(())
}

async fn send_wire_message(socket: &mut TestSocket, message: &WireMessage) -> Result<()> {
    let payload = serde_json::to_string(message).context("serialize wire message")?;
    socket
        .send(Message::Text(payload.into()))
        .await
        .context("send websocket message")
}
