use crate::ServerReadiness;
use crate::agent_logs::AgentLogStats;
#[cfg(test)]
use nodelite_proto::NodeStatus;
use nodelite_proto::{MetricsConfig, NodeIdentity, NodeSnapshot, OverviewData};

mod emitter;
mod node;

use emitter::MetricEmitter;

#[cfg(test)]
pub(crate) fn render_prometheus_metrics(
    readiness: &ServerReadiness,
    statuses: &[NodeStatus],
    overview: &OverviewData,
) -> String {
    render_prometheus_metrics_from_iter(
        readiness,
        statuses.iter().map(PrometheusNode::from_status),
        overview,
        MetricsConfig::default(),
        None,
    )
}

pub(crate) fn render_prometheus_metrics_from_iter<'a>(
    readiness: &ServerReadiness,
    statuses: impl IntoIterator<Item = PrometheusNode<'a>>,
    overview: &OverviewData,
    metrics_config: MetricsConfig,
    string_pool_size: Option<usize>,
) -> String {
    let mut emitter = MetricEmitter::default();
    render_server_metrics(&mut emitter, readiness, string_pool_size);
    render_overview_metrics(&mut emitter, overview);
    for status in statuses {
        node::render_node_metrics(&mut emitter, status, metrics_config);
    }
    emitter.finish()
}

#[derive(Clone, Copy)]
pub(crate) struct PrometheusNode<'a> {
    pub(crate) identity: &'a NodeIdentity,
    pub(crate) snapshot: Option<&'a NodeSnapshot>,
    pub(crate) last_seen: Option<chrono::DateTime<chrono::Utc>>,
    pub(crate) latency_ms: Option<u64>,
    pub(crate) online: bool,
}

impl<'a> PrometheusNode<'a> {
    #[cfg(test)]
    pub(crate) fn from_status(status: &'a NodeStatus) -> Self {
        Self {
            identity: &status.identity,
            snapshot: status.snapshot.as_ref(),
            last_seen: status.last_seen,
            latency_ms: status.latency_ms,
            online: status.online,
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) struct WriterMetrics {
    pub(crate) history_dropped_writes: u64,
    pub(crate) history_queue_depth: u64,
    pub(crate) history_queue_capacity: u64,
    pub(crate) audit_dropped_writes: u64,
    pub(crate) audit_queue_depth: u64,
    pub(crate) audit_queue_capacity: u64,
    pub(crate) audit_write_failures: u64,
    pub(crate) session_control_queue_full_total: u64,
}

pub(crate) fn render_writer_metrics(metrics: WriterMetrics) -> String {
    let mut emitter = MetricEmitter::default();
    render_bounded_queue_metrics(
        &mut emitter,
        BoundedQueueMetrics {
            dropped_metric: "nodelite_history_dropped_writes_total",
            dropped_help: "Number of history samples dropped because the writer queue was full.",
            depth_metric: "nodelite_history_queue_depth",
            depth_help: "Number of history samples waiting in the writer queue.",
            capacity_metric: "nodelite_history_queue_capacity",
            capacity_help: "Maximum number of history samples accepted by the writer queue.",
            dropped_total: metrics.history_dropped_writes,
            depth: metrics.history_queue_depth,
            capacity: metrics.history_queue_capacity,
        },
    );
    render_bounded_queue_metrics(
        &mut emitter,
        BoundedQueueMetrics {
            dropped_metric: "nodelite_audit_dropped_writes_total",
            dropped_help: "Number of audit events dropped because the writer queue was full.",
            depth_metric: "nodelite_audit_queue_depth",
            depth_help: "Number of audit commands waiting in the writer queue.",
            capacity_metric: "nodelite_audit_queue_capacity",
            capacity_help: "Maximum number of audit commands accepted by the writer queue.",
            dropped_total: metrics.audit_dropped_writes,
            depth: metrics.audit_queue_depth,
            capacity: metrics.audit_queue_capacity,
        },
    );
    emitter.counter(
        "nodelite_audit_write_failures_total",
        "Number of audit writer failures while enqueueing or persisting events.",
        &[],
        metrics.audit_write_failures,
    );
    emitter.counter(
        "nodelite_session_control_queue_full_total",
        "Number of session control commands rejected because a bounded queue was full.",
        &[],
        metrics.session_control_queue_full_total,
    );
    emitter.finish()
}

pub(crate) fn render_agent_log_metrics(stats: AgentLogStats) -> String {
    let mut emitter = MetricEmitter::default();
    for (metric, help, value) in [
        (
            "nodelite_agent_log_nodes",
            "Number of nodes currently holding in-memory agent logs.",
            stats.nodes,
        ),
        (
            "nodelite_agent_log_entries",
            "Number of in-memory agent log entries currently retained.",
            stats.entries,
        ),
        (
            "nodelite_agent_log_estimated_bytes",
            "Estimated bytes currently retained by the in-memory agent log store.",
            stats.estimated_bytes,
        ),
    ] {
        emitter.gauge(metric, help, &[], value);
    }
    emitter.finish()
}

#[derive(Clone, Copy)]
pub(crate) struct ApiCacheMetrics {
    pub(crate) nodes_hits: u64,
    pub(crate) nodes_misses: u64,
    pub(crate) nodes_body_bytes: u64,
    pub(crate) overview_hits: u64,
    pub(crate) overview_misses: u64,
    pub(crate) overview_body_bytes: u64,
    pub(crate) metrics_hits: u64,
    pub(crate) metrics_misses: u64,
    pub(crate) metrics_body_bytes: u64,
}

pub(crate) fn render_api_cache_metrics(metrics: ApiCacheMetrics) -> String {
    let mut emitter = MetricEmitter::default();
    for (kind, hits, misses, body_bytes) in [
        (
            "nodes",
            metrics.nodes_hits,
            metrics.nodes_misses,
            metrics.nodes_body_bytes,
        ),
        (
            "overview",
            metrics.overview_hits,
            metrics.overview_misses,
            metrics.overview_body_bytes,
        ),
        (
            "metrics",
            metrics.metrics_hits,
            metrics.metrics_misses,
            metrics.metrics_body_bytes,
        ),
    ] {
        render_cache_hit_metrics(
            &mut emitter,
            CacheHitMetrics {
                hits_metric: "nodelite_api_cache_hits_total",
                hits_help: "Number of cached API response body hits.",
                misses_metric: "nodelite_api_cache_misses_total",
                misses_help: "Number of cached API response body misses.",
            },
            kind,
            hits,
            misses,
        );
        emitter.gauge(
            "nodelite_api_body_bytes",
            "Size in bytes of the most recently built cached API response body.",
            &[("kind", kind)],
            body_bytes,
        );
        render_cache_hit_metrics(
            &mut emitter,
            CacheHitMetrics {
                hits_metric: "nodelite_view_cache_hits_total",
                hits_help: "Number of cached HTTP view response body hits.",
                misses_metric: "nodelite_view_cache_misses_total",
                misses_help: "Number of cached HTTP view response body misses.",
            },
            kind,
            hits,
            misses,
        );
    }
    emitter.gauge(
        "nodelite_metrics_body_bytes",
        "Size in bytes of the most recently built cached base /metrics response body.",
        &[],
        metrics.metrics_body_bytes,
    );
    emitter.finish()
}

#[derive(Clone, Copy)]
struct BoundedQueueMetrics {
    dropped_metric: &'static str,
    dropped_help: &'static str,
    depth_metric: &'static str,
    depth_help: &'static str,
    capacity_metric: &'static str,
    capacity_help: &'static str,
    dropped_total: u64,
    depth: u64,
    capacity: u64,
}

#[derive(Clone, Copy)]
struct CacheHitMetrics {
    hits_metric: &'static str,
    hits_help: &'static str,
    misses_metric: &'static str,
    misses_help: &'static str,
}

fn render_bounded_queue_metrics(emitter: &mut MetricEmitter, metrics: BoundedQueueMetrics) {
    emitter.counter(
        metrics.dropped_metric,
        metrics.dropped_help,
        &[],
        metrics.dropped_total,
    );
    emitter.gauge(metrics.depth_metric, metrics.depth_help, &[], metrics.depth);
    emitter.gauge(
        metrics.capacity_metric,
        metrics.capacity_help,
        &[],
        metrics.capacity,
    );
}

fn render_cache_hit_metrics(
    emitter: &mut MetricEmitter,
    metrics: CacheHitMetrics,
    kind: &str,
    hits: u64,
    misses: u64,
) {
    let labels = [("kind", kind)];
    emitter.counter(metrics.hits_metric, metrics.hits_help, &labels, hits);
    emitter.counter(metrics.misses_metric, metrics.misses_help, &labels, misses);
}

#[derive(Clone, Copy)]
pub(crate) struct WsMessageMetrics {
    pub(crate) metrics_total: u64,
    pub(crate) agent_logs_total: u64,
    pub(crate) pong_total: u64,
    pub(crate) refresh_token_request_total: u64,
}

#[derive(Clone, Copy, Default)]
pub(crate) struct SqliteWalCheckpointStats {
    pub(crate) observed: bool,
    pub(crate) active: bool,
    pub(crate) busy: u64,
    pub(crate) log_pages: u64,
    pub(crate) checkpointed_pages: u64,
}

impl SqliteWalCheckpointStats {
    fn backlog_pages(self) -> u64 {
        self.log_pages.saturating_sub(self.checkpointed_pages)
    }
}

#[derive(Clone, Copy, Default)]
pub(crate) struct SqliteWalCheckpointMetrics {
    pub(crate) history: SqliteWalCheckpointStats,
    pub(crate) audit: SqliteWalCheckpointStats,
}

#[derive(Clone, Copy)]
pub(crate) struct RuntimeMetrics {
    pub(crate) process_resident_memory_bytes: Option<u64>,
    pub(crate) history_db_bytes: u64,
    pub(crate) history_wal_bytes: u64,
    pub(crate) history_shm_bytes: u64,
    pub(crate) audit_db_bytes: u64,
    pub(crate) audit_wal_bytes: u64,
    pub(crate) audit_shm_bytes: u64,
    pub(crate) sqlite_wal_checkpoint: SqliteWalCheckpointMetrics,
    pub(crate) registry_nodes: u64,
    pub(crate) registry_disk_entries_total: u64,
    pub(crate) ws_messages: WsMessageMetrics,
}

pub(crate) fn render_runtime_metrics(metrics: RuntimeMetrics) -> String {
    let mut emitter = MetricEmitter::default();
    if let Some(bytes) = metrics.process_resident_memory_bytes {
        emitter.gauge(
            "nodelite_process_resident_memory_bytes",
            "Resident set size of the nodelite-server process.",
            &[],
            bytes,
        );
    }
    for (kind, bytes) in [
        ("history_db", metrics.history_db_bytes),
        ("history_wal", metrics.history_wal_bytes),
        ("history_shm", metrics.history_shm_bytes),
        ("audit_db", metrics.audit_db_bytes),
        ("audit_wal", metrics.audit_wal_bytes),
        ("audit_shm", metrics.audit_shm_bytes),
    ] {
        emitter.gauge(
            "nodelite_sqlite_file_bytes",
            "Size in bytes of NodeLite SQLite database and journal artifacts.",
            &[("kind", kind)],
            bytes,
        );
    }
    emitter.gauge(
        "nodelite_history_db_bytes",
        "Size in bytes of the history SQLite database file.",
        &[],
        metrics.history_db_bytes,
    );
    emitter.gauge(
        "nodelite_history_wal_bytes",
        "Size in bytes of the history SQLite WAL file.",
        &[],
        metrics.history_wal_bytes,
    );
    render_sqlite_wal_checkpoint_metrics(&mut emitter, metrics.sqlite_wal_checkpoint);
    emitter.gauge(
        "nodelite_registry_nodes",
        "Number of registered nodes currently loaded in memory.",
        &[],
        metrics.registry_nodes,
    );
    emitter.gauge(
        "nodelite_registry_disk_entries_total",
        "Number of disk entries currently held across node snapshots.",
        &[],
        metrics.registry_disk_entries_total,
    );
    for (kind, total) in [
        ("metrics", metrics.ws_messages.metrics_total),
        ("agent_logs", metrics.ws_messages.agent_logs_total),
        ("pong", metrics.ws_messages.pong_total),
        (
            "refresh_token_request",
            metrics.ws_messages.refresh_token_request_total,
        ),
    ] {
        emitter.counter(
            "nodelite_ws_messages_total",
            "Number of authenticated WebSocket messages handled by type.",
            &[("type", kind)],
            total,
        );
    }
    emitter.finish()
}

fn render_sqlite_wal_checkpoint_metrics(
    emitter: &mut MetricEmitter,
    metrics: SqliteWalCheckpointMetrics,
) {
    for (database, stats) in [("history", metrics.history), ("audit", metrics.audit)] {
        emitter.gauge(
            "nodelite_sqlite_wal_checkpoint_observed",
            "Whether the latest passive SQLite WAL checkpoint probe succeeded.",
            &[("database", database)],
            if stats.observed { 1 } else { 0 },
        );
        emitter.gauge(
            "nodelite_sqlite_wal_checkpoint_active",
            "Whether the SQLite database is currently using WAL journal mode.",
            &[("database", database)],
            if stats.active { 1 } else { 0 },
        );
        emitter.gauge(
            "nodelite_sqlite_wal_checkpoint_busy",
            "Busy flag returned by PRAGMA wal_checkpoint(PASSIVE).",
            &[("database", database)],
            stats.busy,
        );
        for (state, pages) in [
            ("log", stats.log_pages),
            ("checkpointed", stats.checkpointed_pages),
            ("backlog", stats.backlog_pages()),
        ] {
            emitter.gauge(
                "nodelite_sqlite_wal_checkpoint_pages",
                "SQLite WAL checkpoint page counts from PRAGMA wal_checkpoint(PASSIVE).",
                &[("database", database), ("state", state)],
                pages,
            );
        }
    }
}

pub(crate) fn render_metrics_response_body_bytes(bytes: u64) -> String {
    let mut emitter = MetricEmitter::default();
    emitter.gauge(
        "nodelite_metrics_response_body_bytes",
        "Size in bytes of the uncompressed /metrics response body before HTTP compression.",
        &[],
        bytes,
    );
    emitter.finish()
}

fn render_server_metrics(
    emitter: &mut MetricEmitter,
    readiness: &ServerReadiness,
    string_pool_size: Option<usize>,
) {
    emitter.gauge(
        "nodelite_server_ready",
        "Whether the NodeLite server is ready to serve protected traffic.",
        &[],
        if readiness.is_ready() { 1 } else { 0 },
    );
    emitter.gauge(
        "nodelite_history_available",
        "Whether the history store is currently available.",
        &[],
        if readiness.history_available() { 1 } else { 0 },
    );
    emitter.gauge(
        "nodelite_registry_reload_healthy",
        "Whether the registry reload loop is currently healthy.",
        &[],
        if readiness.registry_reload_healthy() {
            1
        } else {
            0
        },
    );
    if let Some(size) = string_pool_size {
        emitter.gauge(
            "nodelite_string_pool_entries",
            "Number of unique strings in the interning pool (monotonically increasing).",
            &[],
            size as i64,
        );
    }
}

fn render_overview_metrics(emitter: &mut MetricEmitter, overview: &OverviewData) {
    emitter.gauge(
        "nodelite_nodes_total",
        "Number of registered nodes known to the dashboard.",
        &[],
        overview.total_nodes,
    );
    emitter.gauge(
        "nodelite_nodes_online",
        "Number of nodes currently considered online.",
        &[],
        overview.online_nodes,
    );
    emitter.gauge(
        "nodelite_nodes_offline",
        "Number of nodes currently considered offline.",
        &[],
        overview.offline_nodes,
    );
    emitter.counter(
        "nodelite_network_total_bytes",
        "Summed network byte counters reported by all nodes.",
        &[("direction", "rx")],
        overview.total_rx_bytes,
    );
    emitter.counter(
        "nodelite_network_total_bytes",
        "Summed network byte counters reported by all nodes.",
        &[("direction", "tx")],
        overview.total_tx_bytes,
    );
    emitter.gauge(
        "nodelite_network_rate_bytes_per_second",
        "Summed instantaneous network rate reported by all nodes.",
        &[("direction", "rx")],
        overview.current_rx_bytes_per_sec,
    );
    emitter.gauge(
        "nodelite_network_rate_bytes_per_second",
        "Summed instantaneous network rate reported by all nodes.",
        &[("direction", "tx")],
        overview.current_tx_bytes_per_sec,
    );
    if let Some(average_latency_ms) = overview.average_latency_ms
        && average_latency_ms.is_finite()
    {
        emitter.gauge(
            "nodelite_latency_average_milliseconds",
            "Average latency across online nodes.",
            &[],
            average_latency_ms,
        );
    }
}

#[cfg(test)]
mod tests;
