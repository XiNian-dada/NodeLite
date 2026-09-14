//! Fresh-process RSS checks include allocator retention under dense and sparse log workloads.

use anyhow::{Context, Result, ensure};
use chrono::Utc;
use nodelite_proto::{AgentLogEntry, AgentLogsConfig, NoticeLevel};
use nodelite_server::bench_support::AgentLogStore;

use super::diagnostics::current_rss_bytes;

pub(super) async fn run(sparse: bool) -> Result<()> {
    let budget = std::env::var("NODELITE_LOG_BUDGET_BYTES")
        .map(|value| value.parse::<usize>())
        .unwrap_or(Ok(8 * 1024 * 1024))
        .context("parse NODELITE_LOG_BUDGET_BYTES")?;
    ensure!(
        (65536..=67108864).contains(&budget),
        "log budget outside supported bounds"
    );
    let store = AgentLogStore::with_limits(AgentLogsConfig {
        max_entries: 10_000,
        max_estimated_bytes: budget,
    });
    let baseline = current_rss_bytes()?;
    let mut peak = baseline;
    let nodes = if sparse { 12_000 } else { 200 };
    let batch = if sparse { 1 } else { 64 };
    for round in 0..4 {
        for node in 0..nodes {
            store
                .record_entries(
                    &format!("log-node-{node:05}"),
                    (0..batch)
                        .map(|_| AgentLogEntry {
                            occurred_at: Utc::now().to_rfc3339(),
                            level: NoticeLevel::Info,
                            message: if sparse {
                                format!("round-{round}")
                            } else {
                                "x".repeat(512)
                            },
                        })
                        .collect(),
                )
                .await;
            if node % 32 == 0 {
                peak = peak.max(current_rss_bytes()?);
            }
        }
    }
    peak = peak.max(current_rss_bytes()?);
    let stats = store.stats().await;
    let delta = peak.saturating_sub(baseline);
    // Runtime page faults and allocator arenas can add a small constant beyond retained logs.
    let tolerance = 1024 * 1024;
    println!(
        "LOG_MEMORY_RESULT layout={} budget_bytes={} entries={} estimated_bytes={} evictions={} baseline_rss_bytes={} peak_rss_bytes={} rss_delta_bytes={} tolerance_bytes={}",
        if sparse { "sparse" } else { "dense" },
        budget,
        stats.entries,
        stats.estimated_bytes,
        stats.evicted_global_budget_total,
        baseline,
        peak,
        delta,
        tolerance
    );
    ensure!(
        stats.evicted_global_budget_total > 0,
        "workload must exercise the global budget"
    );
    ensure!(
        stats.entries <= stats.max_entries && stats.estimated_bytes <= budget,
        "log budget exceeded"
    );
    ensure!(
        delta <= budget as u64 + tolerance,
        "log RSS exceeds budget plus 1 MiB tolerance"
    );
    store.forget_missing(&[]).await;
    ensure!(
        store.stats().await.estimated_bytes == 0,
        "cleanup must release retained log allocations"
    );
    Ok(())
}
