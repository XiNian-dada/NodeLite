//! 概览聚合与数值安全辅助逻辑。

use chrono::Utc;
#[cfg(test)]
use nodelite_proto::NodeStatus;
use nodelite_proto::{NodeSnapshot, OverviewData};

#[derive(Clone, Copy)]
pub(super) struct OverviewNode<'a> {
    pub(super) online: bool,
    pub(super) latency_ms: Option<u64>,
    pub(super) snapshot: Option<&'a NodeSnapshot>,
}

impl<'a> OverviewNode<'a> {
    #[cfg(test)]
    pub(super) fn from_status(status: &'a NodeStatus) -> Self {
        Self {
            online: status.online,
            latency_ms: status.latency_ms,
            snapshot: status.snapshot.as_ref(),
        }
    }
}

pub(super) fn build_overview_from_iter<'a>(
    statuses: impl IntoIterator<Item = OverviewNode<'a>>,
) -> OverviewData {
    let mut total_nodes = 0_usize;
    let mut online_nodes = 0_usize;
    let mut total_rx_bytes = 0_u64;
    let mut total_tx_bytes = 0_u64;
    let mut current_rx_bytes_per_sec = 0.0;
    let mut current_tx_bytes_per_sec = 0.0;
    let mut latency_total = 0_u128;
    let mut latency_samples = 0_usize;

    for status in statuses {
        total_nodes += 1;
        if status.online {
            online_nodes += 1;
            if let Some(latency) = status.latency_ms {
                latency_total = latency_total.saturating_add(latency as u128);
                latency_samples += 1;
            }
        }

        let Some(snapshot) = status.snapshot else {
            continue;
        };
        total_rx_bytes = total_rx_bytes.saturating_add(snapshot.network.total_rx_bytes);
        total_tx_bytes = total_tx_bytes.saturating_add(snapshot.network.total_tx_bytes);
        if let Some(rx) = snapshot.network.rx_bytes_per_sec {
            current_rx_bytes_per_sec = sum_finite_f64(current_rx_bytes_per_sec, rx);
        }
        if let Some(tx) = snapshot.network.tx_bytes_per_sec {
            current_tx_bytes_per_sec = sum_finite_f64(current_tx_bytes_per_sec, tx);
        }
    }

    let offline_nodes = total_nodes.saturating_sub(online_nodes);
    let average_latency_ms =
        (latency_samples > 0).then(|| latency_total as f64 / latency_samples as f64);

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

/// 把浮点数累加器中的非法值(NaN / 负值 / 溢出)安全过滤掉。
fn sum_finite_f64(total: f64, value: f64) -> f64 {
    if !value.is_finite() || value < 0.0 {
        return total;
    }

    let next = total + value;
    if next.is_finite() { next } else { f64::MAX }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use nodelite_proto::{
        LoadAverage, MemoryUsage, NetworkCounters, NodeIdentity, NodeSnapshot, NodeStatus,
    };

    use super::{OverviewNode, build_overview_from_iter};

    fn status(
        node_id: &str,
        online: bool,
        latency_ms: Option<u64>,
        snapshot: Option<NodeSnapshot>,
    ) -> NodeStatus {
        NodeStatus {
            identity: NodeIdentity {
                node_id: node_id.to_string(),
                node_label: node_id.to_string(),
                hostname: format!("{node_id}.example"),
                os: "linux".to_string(),
                kernel_version: None,
                cpu_model: None,
                cpu_cores: 4,
                agent_version: "test".to_string(),
                boot_time: None,
                tags: Vec::new(),
            },
            remote_ip: None,
            geoip_country: None,
            geoip_city: None,
            geoip_latitude: None,
            geoip_longitude: None,
            snapshot,
            last_seen: None,
            latency_ms,
            online,
        }
    }

    fn snapshot(
        total_rx_bytes: u64,
        total_tx_bytes: u64,
        rx_bytes_per_sec: Option<f64>,
        tx_bytes_per_sec: Option<f64>,
    ) -> NodeSnapshot {
        NodeSnapshot {
            collected_at: Utc::now(),
            cpu_usage_percent: Some(10.0),
            load: LoadAverage {
                one: 1.0,
                five: 1.0,
                fifteen: 1.0,
            },
            memory: MemoryUsage {
                total_bytes: 100,
                used_bytes: 50,
                available_bytes: 50,
                swap_total_bytes: 0,
                swap_used_bytes: 0,
            },
            uptime_secs: 60,
            disks: Vec::new(),
            network: NetworkCounters {
                total_rx_bytes,
                total_tx_bytes,
                rx_bytes_per_sec,
                tx_bytes_per_sec,
            },
        }
    }

    #[test]
    fn build_overview_counts_online_latency_and_snapshots() {
        let statuses = [
            status(
                "hk-01",
                true,
                Some(120),
                Some(snapshot(10, 20, Some(1.5), Some(2.5))),
            ),
            status("hk-02", false, Some(999), None),
            status(
                "hk-03",
                true,
                Some(60),
                Some(snapshot(30, 40, Some(3.5), Some(4.5))),
            ),
        ];

        let overview = build_overview_from_iter(statuses.iter().map(OverviewNode::from_status));
        assert_eq!(overview.total_nodes, 3);
        assert_eq!(overview.online_nodes, 2);
        assert_eq!(overview.offline_nodes, 1);
        assert_eq!(overview.total_rx_bytes, 40);
        assert_eq!(overview.total_tx_bytes, 60);
        assert_eq!(overview.current_rx_bytes_per_sec, 5.0);
        assert_eq!(overview.current_tx_bytes_per_sec, 7.0);
        assert_eq!(overview.average_latency_ms, Some(90.0));
    }

    #[test]
    fn build_overview_ignores_invalid_rates_and_clamps_overflow() {
        let statuses = [
            status(
                "hk-01",
                true,
                None,
                Some(snapshot(0, 0, Some(-1.0), Some(f64::MAX))),
            ),
            status(
                "hk-02",
                true,
                None,
                Some(snapshot(0, 0, Some(f64::INFINITY), Some(f64::MAX))),
            ),
        ];

        let overview = build_overview_from_iter(statuses.iter().map(OverviewNode::from_status));
        assert_eq!(overview.current_rx_bytes_per_sec, 0.0);
        assert_eq!(overview.current_tx_bytes_per_sec, f64::MAX);
        assert_eq!(overview.average_latency_ms, None);
    }
}
