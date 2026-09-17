//! Live log budgets and losses stay observable independently of cached node metrics.

use super::emitter::MetricEmitter;
use crate::agent_logs::AgentLogStats;

pub(crate) fn render_agent_log_metrics(stats: AgentLogStats) -> String {
    let mut emitter = MetricEmitter::default();
    for (name, help, value) in [
        (
            "nodelite_agent_logs_nodes",
            "Nodes with buffered Agent logs.",
            stats.nodes,
        ),
        (
            "nodelite_agent_logs_entries",
            "Currently buffered Agent log entries.",
            stats.entries,
        ),
        (
            "nodelite_agent_logs_estimated_bytes",
            "Estimated allocation retained by Agent logs.",
            stats.estimated_bytes,
        ),
        (
            "nodelite_agent_logs_max_entries",
            "Configured global Agent log entry budget.",
            stats.max_entries,
        ),
        (
            "nodelite_agent_logs_max_estimated_bytes",
            "Configured Agent log allocation budget.",
            stats.max_estimated_bytes,
        ),
    ] {
        emitter.gauge(name, help, &[], value);
    }
    for (reason, count) in [
        ("global_budget", stats.evicted_global_budget_total),
        ("node_limit", stats.evicted_per_node_total),
    ] {
        emitter.counter(
            "nodelite_agent_logs_evictions_total",
            "Agent log entries evicted to honor a retention budget.",
            &[("reason", reason)],
            count,
        );
    }
    emitter.counter(
        "nodelite_agent_logs_dropped_total",
        "Agent log entries rejected by batch or input limits.",
        &[],
        stats.dropped_total,
    );
    emitter.finish()
}
