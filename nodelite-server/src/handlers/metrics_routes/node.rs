use nodelite_proto::{MetricsConfig, NodeSnapshot};

use super::PrometheusNode;
use super::emitter::MetricEmitter;

pub(super) fn render_node_metrics(
    emitter: &mut MetricEmitter,
    status: PrometheusNode<'_>,
    config: MetricsConfig,
) {
    let node_id = status.identity.node_id.as_str();
    let node_labels = [("node_id", node_id)];
    let node_info_labels = [
        ("node_id", node_id),
        ("node_label", status.identity.node_label.as_str()),
        ("hostname", status.identity.hostname.as_str()),
        ("os", status.identity.os.as_str()),
        ("agent_version", status.identity.agent_version.as_str()),
    ];

    emitter.gauge(
        "nodelite_node_info",
        "Static node metadata exposed as an info metric.",
        &node_info_labels,
        1,
    );
    emitter.gauge(
        "nodelite_node_online",
        "Whether the node is currently online.",
        &node_labels,
        if status.online { 1 } else { 0 },
    );
    if let Some(last_seen) = status.last_seen {
        emitter.gauge(
            "nodelite_node_last_seen_timestamp_seconds",
            "Last time the node was seen by the server as a Unix timestamp.",
            &node_labels,
            last_seen.timestamp(),
        );
    }
    if let Some(latency_ms) = status.latency_ms {
        emitter.gauge(
            "nodelite_node_latency_milliseconds",
            "Latest measured node latency in milliseconds.",
            &node_labels,
            latency_ms,
        );
    }
    if let Some(snapshot) = status.snapshot.as_ref() {
        render_snapshot_metrics(emitter, node_id, snapshot, config);
    }
}

fn render_snapshot_metrics(
    emitter: &mut MetricEmitter,
    node_id: &str,
    snapshot: &NodeSnapshot,
    config: MetricsConfig,
) {
    let node_labels = [("node_id", node_id)];
    if config.export_node_resource_metrics {
        emitter.gauge(
            "nodelite_node_snapshot_timestamp_seconds",
            "Collection time of the latest node snapshot as a Unix timestamp.",
            &node_labels,
            snapshot.collected_at.timestamp(),
        );
        emitter.gauge(
            "nodelite_node_uptime_seconds",
            "Node uptime in seconds from the latest snapshot.",
            &node_labels,
            snapshot.uptime_secs,
        );
        if let Some(cpu_usage_percent) =
            snapshot.cpu_usage_percent.filter(|value| value.is_finite())
        {
            emitter.gauge(
                "nodelite_node_cpu_usage_ratio",
                "Latest CPU usage ratio reported by the node in the range 0..1.",
                &node_labels,
                cpu_usage_percent / 100.0,
            );
        }
        render_memory_metrics(emitter, node_id, snapshot);
        render_load_metrics(emitter, node_id, snapshot);
        render_network_metrics(emitter, node_id, snapshot);
    }
    if config.export_node_disk_metrics {
        render_disk_metrics(emitter, node_id, snapshot);
    }
}

fn render_memory_metrics(emitter: &mut MetricEmitter, node_id: &str, snapshot: &NodeSnapshot) {
    for (state, value) in [
        ("total", snapshot.memory.total_bytes),
        ("used", snapshot.memory.used_bytes),
        ("available", snapshot.memory.available_bytes),
    ] {
        emitter.gauge(
            "nodelite_node_memory_bytes",
            "Latest memory totals reported by the node.",
            &[("node_id", node_id), ("state", state)],
            value,
        );
    }
}

fn render_load_metrics(emitter: &mut MetricEmitter, node_id: &str, snapshot: &NodeSnapshot) {
    for (window, value) in [
        ("1m", snapshot.load.one),
        ("5m", snapshot.load.five),
        ("15m", snapshot.load.fifteen),
    ] {
        emitter.gauge(
            "nodelite_node_load_average",
            "Latest node load average window.",
            &[("node_id", node_id), ("window", window)],
            value,
        );
    }
}

fn render_network_metrics(emitter: &mut MetricEmitter, node_id: &str, snapshot: &NodeSnapshot) {
    for (direction, value) in [
        ("rx", snapshot.network.total_rx_bytes),
        ("tx", snapshot.network.total_tx_bytes),
    ] {
        emitter.counter(
            "nodelite_node_network_bytes_total",
            "Latest aggregate network byte counters reported by the node.",
            &[("node_id", node_id), ("direction", direction)],
            value,
        );
    }
    for (direction, value) in [
        ("rx", snapshot.network.rx_bytes_per_sec),
        ("tx", snapshot.network.tx_bytes_per_sec),
    ] {
        if let Some(value) = value.filter(|value| value.is_finite()) {
            emitter.gauge(
                "nodelite_node_network_rate_bytes_per_second",
                "Latest aggregate network transfer rate reported by the node.",
                &[("node_id", node_id), ("direction", direction)],
                value,
            );
        }
    }
    if let Some(packet_loss_percent) = snapshot
        .network
        .packet_loss_percent
        .filter(|value| value.is_finite())
    {
        emitter.gauge(
            "nodelite_node_network_packet_loss_ratio",
            "Latest aggregate network packet loss ratio reported by the node in the range 0..1.",
            &[("node_id", node_id)],
            packet_loss_percent / 100.0,
        );
    }
}

fn render_disk_metrics(emitter: &mut MetricEmitter, node_id: &str, snapshot: &NodeSnapshot) {
    for disk in &snapshot.disks {
        for (state, value) in [
            ("total", disk.total_bytes),
            ("used", disk.used_bytes),
            ("available", disk.available_bytes),
        ] {
            emitter.gauge(
                "nodelite_node_disk_bytes",
                "Latest disk byte totals reported by the node.",
                &[
                    ("node_id", node_id),
                    ("mount_point", &disk.mount_point),
                    ("state", state),
                ],
                value,
            );
        }
        if disk.used_percent.is_finite() {
            emitter.gauge(
                "nodelite_node_disk_used_ratio",
                "Latest disk used ratio reported by the node in the range 0..1.",
                &[("node_id", node_id), ("mount_point", &disk.mount_point)],
                disk.used_percent / 100.0,
            );
        }
    }
}
