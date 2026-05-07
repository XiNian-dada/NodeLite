use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use chrono::{DateTime, Utc};
use tokio::sync::RwLock;
use ximonitor_proto::{NodeIdentity, NodeSnapshot, NodeStatus, OverviewData, ServerConfig};

#[derive(Clone)]
pub struct SharedState {
    config: Arc<ServerConfig>,
    registry: Arc<RwLock<Registry>>,
    next_session_id: Arc<AtomicU64>,
}

impl SharedState {
    pub fn new(config: Arc<ServerConfig>) -> Self {
        Self {
            config,
            registry: Arc::new(RwLock::new(Registry::default())),
            next_session_id: Arc::new(AtomicU64::new(1)),
        }
    }

    pub fn config(&self) -> &ServerConfig {
        self.config.as_ref()
    }

    pub async fn register_node(&self, identity: NodeIdentity) -> u64 {
        let session_id = self.next_session_id.fetch_add(1, Ordering::Relaxed);
        let now = Utc::now();
        let mut registry = self.registry.write().await;
        registry.register_node(session_id, identity, now);
        session_id
    }

    pub async fn update_snapshot(
        &self,
        node_id: &str,
        session_id: u64,
        snapshot: NodeSnapshot,
    ) -> bool {
        let mut registry = self.registry.write().await;
        registry.update_snapshot(node_id, session_id, snapshot, Utc::now())
    }

    pub async fn update_latency(&self, node_id: &str, session_id: u64, latency_ms: u64) -> bool {
        let mut registry = self.registry.write().await;
        registry.update_latency(node_id, session_id, latency_ms, Utc::now())
    }

    pub async fn mark_disconnected(&self, node_id: &str, session_id: u64) {
        let mut registry = self.registry.write().await;
        registry.mark_disconnected(node_id, session_id);
    }

    pub async fn mark_stale(&self) -> usize {
        let mut registry = self.registry.write().await;
        registry.mark_stale(
            Duration::from_secs(self.config.stale_after_secs),
            Utc::now(),
        )
    }

    pub async fn is_current_session(&self, node_id: &str, session_id: u64) -> bool {
        let registry = self.registry.read().await;
        registry.is_current_session(node_id, session_id)
    }

    pub async fn node_count(&self) -> usize {
        let registry = self.registry.read().await;
        registry.nodes.len()
    }

    pub async fn list_statuses(&self) -> Vec<NodeStatus> {
        let registry = self.registry.read().await;
        registry.list_statuses()
    }

    pub async fn get_status(&self, node_id: &str) -> Option<NodeStatus> {
        let registry = self.registry.read().await;
        registry.get_status(node_id)
    }

    pub async fn overview(&self) -> OverviewData {
        let registry = self.registry.read().await;
        registry.overview()
    }

    pub async fn restore_statuses(&self, statuses: Vec<NodeStatus>) {
        let mut registry = self.registry.write().await;
        registry.restore_statuses(statuses);
    }
}

#[derive(Debug, Default)]
struct Registry {
    nodes: HashMap<String, NodeEntry>,
}

#[derive(Debug, Clone)]
struct NodeEntry {
    status: NodeStatus,
    active_session_id: Option<u64>,
}

impl Registry {
    fn register_node(&mut self, session_id: u64, identity: NodeIdentity, now: DateTime<Utc>) {
        let node_id = identity.node_id.clone();
        let entry = self.nodes.entry(node_id).or_insert_with(|| NodeEntry {
            status: NodeStatus {
                identity: identity.clone(),
                snapshot: None,
                last_seen: Some(now),
                latency_ms: None,
                online: true,
            },
            active_session_id: Some(session_id),
        });

        entry.status.identity = identity;
        entry.status.online = true;
        entry.status.last_seen = Some(now);
        entry.status.latency_ms = None;
        entry.active_session_id = Some(session_id);
    }

    fn update_snapshot(
        &mut self,
        node_id: &str,
        session_id: u64,
        snapshot: NodeSnapshot,
        now: DateTime<Utc>,
    ) -> bool {
        let Some(entry) = self.nodes.get_mut(node_id) else {
            return false;
        };
        if entry.active_session_id != Some(session_id) {
            return false;
        }

        entry.status.snapshot = Some(snapshot);
        entry.status.last_seen = Some(now);
        entry.status.online = true;
        true
    }

    fn update_latency(
        &mut self,
        node_id: &str,
        session_id: u64,
        latency_ms: u64,
        now: DateTime<Utc>,
    ) -> bool {
        let Some(entry) = self.nodes.get_mut(node_id) else {
            return false;
        };
        if entry.active_session_id != Some(session_id) {
            return false;
        }

        entry.status.latency_ms = Some(latency_ms);
        entry.status.last_seen = Some(now);
        entry.status.online = true;
        true
    }

    fn mark_disconnected(&mut self, node_id: &str, session_id: u64) {
        let Some(entry) = self.nodes.get_mut(node_id) else {
            return;
        };
        if entry.active_session_id == Some(session_id) {
            entry.active_session_id = None;
            entry.status.online = false;
        }
    }

    fn mark_stale(&mut self, threshold: Duration, now: DateTime<Utc>) -> usize {
        let mut marked = 0;

        for entry in self.nodes.values_mut() {
            let Some(last_seen) = entry.status.last_seen else {
                continue;
            };
            let Ok(elapsed) = (now - last_seen).to_std() else {
                continue;
            };
            if elapsed >= threshold && entry.status.online {
                entry.status.online = false;
                entry.active_session_id = None;
                marked += 1;
            }
        }

        marked
    }

    fn is_current_session(&self, node_id: &str, session_id: u64) -> bool {
        self.nodes
            .get(node_id)
            .and_then(|entry| entry.active_session_id)
            == Some(session_id)
    }

    fn list_statuses(&self) -> Vec<NodeStatus> {
        let mut statuses: Vec<NodeStatus> = self
            .nodes
            .values()
            .map(|entry| entry.status.clone())
            .collect();
        statuses.sort_by(|left, right| {
            left.identity
                .node_label
                .cmp(&right.identity.node_label)
                .then_with(|| left.identity.node_id.cmp(&right.identity.node_id))
        });
        statuses
    }

    fn get_status(&self, node_id: &str) -> Option<NodeStatus> {
        self.nodes.get(node_id).map(|entry| entry.status.clone())
    }

    fn overview(&self) -> OverviewData {
        let statuses = self.list_statuses();
        let total_nodes = statuses.len();
        let online_nodes = statuses.iter().filter(|status| status.online).count();
        let offline_nodes = total_nodes.saturating_sub(online_nodes);
        let total_rx_bytes = statuses
            .iter()
            .filter_map(|status| status.snapshot.as_ref())
            .map(|snapshot| snapshot.network.total_rx_bytes)
            .sum();
        let total_tx_bytes = statuses
            .iter()
            .filter_map(|status| status.snapshot.as_ref())
            .map(|snapshot| snapshot.network.total_tx_bytes)
            .sum();
        let current_rx_bytes_per_sec = statuses
            .iter()
            .filter_map(|status| status.snapshot.as_ref())
            .filter_map(|snapshot| snapshot.network.rx_bytes_per_sec)
            .sum();
        let current_tx_bytes_per_sec = statuses
            .iter()
            .filter_map(|status| status.snapshot.as_ref())
            .filter_map(|snapshot| snapshot.network.tx_bytes_per_sec)
            .sum();

        let latencies: Vec<u64> = statuses
            .iter()
            .filter(|status| status.online)
            .filter_map(|status| status.latency_ms)
            .collect();
        let average_latency_ms = (!latencies.is_empty())
            .then(|| latencies.iter().copied().sum::<u64>() as f64 / latencies.len() as f64);

        OverviewData {
            generated_at: Utc::now(),
            total_nodes,
            online_nodes,
            offline_nodes,
            total_rx_bytes,
            total_tx_bytes,
            current_rx_bytes_per_sec,
            current_tx_bytes_per_sec,
            average_latency_ms,
        }
    }

    fn restore_statuses(&mut self, statuses: Vec<NodeStatus>) {
        self.nodes.clear();
        for mut status in statuses {
            status.online = false;
            let node_id = status.identity.node_id.clone();
            self.nodes.insert(
                node_id,
                NodeEntry {
                    status,
                    active_session_id: None,
                },
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use chrono::{Duration as ChronoDuration, TimeZone, Utc};
    use ximonitor_proto::{LoadAverage, MemoryUsage, NodeSnapshot};
    use ximonitor_proto::{NetworkCounters, percentage};

    use super::Registry;
    use ximonitor_proto::NodeIdentity;

    #[test]
    fn newer_session_replaces_older_one() {
        let mut registry = Registry::default();
        let now = Utc.with_ymd_and_hms(2026, 5, 7, 0, 0, 0).unwrap();
        let identity = NodeIdentity {
            node_id: "hk-01".to_string(),
            node_label: "Hong Kong 01".to_string(),
            hostname: "hk-01".to_string(),
            os: "linux".to_string(),
            kernel_version: None,
            cpu_model: None,
            cpu_cores: 4,
            agent_version: "0.1.0".to_string(),
            boot_time: None,
            tags: Vec::new(),
        };

        registry.register_node(1, identity.clone(), now);
        registry.register_node(2, identity, now + ChronoDuration::seconds(3));

        assert!(!registry.update_snapshot("hk-01", 1, sample_snapshot(now), now));
        assert!(registry.update_snapshot(
            "hk-01",
            2,
            sample_snapshot(now + ChronoDuration::seconds(4)),
            now
        ));
    }

    #[test]
    fn stale_nodes_are_marked_offline() {
        let mut registry = Registry::default();
        let now = Utc.with_ymd_and_hms(2026, 5, 7, 0, 0, 0).unwrap();

        registry.register_node(7, sample_identity(), now);
        assert_eq!(
            registry.mark_stale(Duration::from_secs(10), now + ChronoDuration::seconds(15)),
            1
        );
        assert!(
            !registry
                .list_statuses()
                .first()
                .expect("node status")
                .online
        );
    }

    fn sample_identity() -> NodeIdentity {
        NodeIdentity {
            node_id: "hk-01".to_string(),
            node_label: "Hong Kong 01".to_string(),
            hostname: "hk-01".to_string(),
            os: "linux".to_string(),
            kernel_version: None,
            cpu_model: None,
            cpu_cores: 4,
            agent_version: "0.1.0".to_string(),
            boot_time: None,
            tags: Vec::new(),
        }
    }

    fn sample_snapshot(now: chrono::DateTime<Utc>) -> NodeSnapshot {
        NodeSnapshot {
            collected_at: now,
            cpu_usage_percent: percentage(1, 2),
            load: LoadAverage {
                one: 0.1,
                five: 0.2,
                fifteen: 0.3,
            },
            memory: MemoryUsage {
                total_bytes: 1024,
                used_bytes: 512,
                available_bytes: 256,
                swap_total_bytes: 128,
                swap_used_bytes: 64,
            },
            uptime_secs: 60,
            disks: Vec::new(),
            network: NetworkCounters {
                total_rx_bytes: 100,
                total_tx_bytes: 200,
                rx_bytes_per_sec: Some(5.0),
                tx_bytes_per_sec: Some(7.0),
            },
        }
    }
}
