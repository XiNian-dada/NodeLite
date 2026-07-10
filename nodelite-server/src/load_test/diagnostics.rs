use anyhow::{Context, Result};

use super::server::{HistoryArtifactBytes, TestServer};

#[derive(Debug, Clone, Copy)]
pub(super) struct ViewCacheCounters {
    pub(super) overview_hits: u64,
    pub(super) overview_misses: u64,
    pub(super) nodes_hits: u64,
    pub(super) nodes_misses: u64,
    pub(super) metrics_hits: u64,
    pub(super) metrics_misses: u64,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct ResourceSnapshot {
    pub(super) rss_bytes: u64,
    pub(super) history_queue_depth: usize,
    pub(super) history_dropped_writes: u64,
    pub(super) history_artifacts: HistoryArtifactBytes,
    pub(super) view_cache: ViewCacheCounters,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct ProcessMemorySnapshot {
    pub(super) rss_bytes: u64,
    pub(super) pss_bytes: Option<u64>,
    pub(super) rss_anon_bytes: Option<u64>,
}

impl ResourceSnapshot {
    pub(super) async fn capture(server: &TestServer) -> Result<Self> {
        let (history_queue_depth, _) = server.history.writer_queue_metrics().await;
        let api_metrics = server.shared.api_cache_metrics();
        Ok(Self {
            rss_bytes: current_process_memory()?.rss_bytes,
            history_queue_depth: history_queue_depth as usize,
            history_dropped_writes: server.history.dropped_writes(),
            history_artifacts: server.history_artifact_bytes().await?,
            view_cache: ViewCacheCounters {
                overview_hits: api_metrics.overview_hits,
                overview_misses: api_metrics.overview_misses,
                nodes_hits: api_metrics.nodes_hits,
                nodes_misses: api_metrics.nodes_misses,
                metrics_hits: api_metrics.metrics_hits,
                metrics_misses: api_metrics.metrics_misses,
            },
        })
    }
}

pub(super) fn current_process_memory() -> Result<ProcessMemorySnapshot> {
    let rss_bytes = current_rss_bytes()?;
    #[cfg(target_os = "linux")]
    {
        let pss_bytes = linux_memory_kib("/proc/self/smaps_rollup", "Pss:")?.map(kib_to_bytes);
        let rss_anon_bytes = linux_memory_kib("/proc/self/status", "RssAnon:")?.map(kib_to_bytes);
        return Ok(ProcessMemorySnapshot {
            rss_bytes,
            pss_bytes,
            rss_anon_bytes,
        });
    }

    #[cfg(not(target_os = "linux"))]
    Ok(ProcessMemorySnapshot {
        rss_bytes,
        pss_bytes: None,
        rss_anon_bytes: None,
    })
}

#[cfg(target_os = "linux")]
pub(super) fn current_rss_bytes() -> Result<u64> {
    linux_memory_kib("/proc/self/status", "VmRSS:")?
        .map(kib_to_bytes)
        .context("VmRSS is missing from /proc/self/status")
}

#[cfg(not(target_os = "linux"))]
pub(super) fn current_rss_bytes() -> Result<u64> {
    crate::handlers::process_resident_memory_bytes()
        .context("current platform does not expose process RSS")
}

#[cfg(target_os = "linux")]
fn linux_memory_kib(path: &str, key: &str) -> Result<Option<u64>> {
    let contents = std::fs::read_to_string(path).with_context(|| format!("read {path}"))?;
    let Some(line) = contents.lines().find(|line| line.starts_with(key)) else {
        return Ok(None);
    };
    let value = line
        .split_whitespace()
        .nth(1)
        .with_context(|| format!("parse {key} from {path}"))?
        .parse::<u64>()
        .with_context(|| format!("parse {key} KiB value from {path}"))?;
    Ok(Some(value))
}

#[cfg(target_os = "linux")]
fn kib_to_bytes(value: u64) -> u64 {
    value.saturating_mul(1024)
}
