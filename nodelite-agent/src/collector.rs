//! 主机指标采集器入口:按目标平台分派到具体实现。

use anyhow::{Context, Result};
use nodelite_proto::{AgentConfig, NodeIdentity, NodeSnapshot};

#[cfg(any(target_os = "linux", target_os = "macos"))]
use tracing::warn;

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[path = "collector/shared.rs"]
mod shared;

#[cfg(target_os = "linux")]
#[path = "collector_linux.rs"]
mod collector_linux;
#[cfg(target_os = "linux")]
pub use collector_linux::{HostCollector, new_collector};

#[cfg(target_os = "macos")]
#[path = "collector_macos.rs"]
mod collector_macos;
#[cfg(target_os = "macos")]
pub use collector_macos::{HostCollector, new_collector};

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
#[path = "collector_unsupported.rs"]
mod collector_unsupported;
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub use collector_unsupported::{HostCollector, new_collector};

/// Collect node identity on Tokio's blocking pool so startup does not run filesystem or FFI work
/// on an async worker.
pub async fn collect_identity_blocking(
    collector: &mut HostCollector,
    config: AgentConfig,
    agent_version: String,
) -> Result<NodeIdentity> {
    with_collector_blocking(collector, move |collector| {
        collector.collect_identity(&config, &agent_version)
    })
    .await
}

/// Collect a host snapshot on Tokio's blocking pool while preserving collector delta state.
pub async fn collect_snapshot_blocking(collector: &mut HostCollector) -> Result<NodeSnapshot> {
    with_collector_blocking(collector, HostCollector::collect_snapshot).await
}

async fn with_collector_blocking<T, F>(collector: &mut HostCollector, operation: F) -> Result<T>
where
    T: Send + 'static,
    F: FnOnce(&mut HostCollector) -> Result<T> + Send + 'static,
{
    let mut owned = std::mem::replace(collector, new_collector());
    let (returned, result) = tokio::task::spawn_blocking(move || {
        let result = operation(&mut owned);
        (owned, result)
    })
    .await
    .context("collector blocking task failed")?;
    *collector = returned;
    result
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn log_network_rate_anomalies(anomalies: shared::NetworkRateAnomalies) {
    for anomaly in [anomalies.rx, anomalies.tx].into_iter().flatten() {
        warn!(
            direction = anomaly.direction.as_str(),
            rate_bytes_per_sec = anomaly.rate_bytes_per_sec,
            baseline_avg_bytes_per_sec = anomaly.baseline_avg_bytes_per_sec,
            effective_baseline_bytes_per_sec = anomaly.effective_baseline_bytes_per_sec,
            multiplier = anomaly.multiplier,
            sample_count = anomaly.sample_count,
            "network rate is more than 100x above recent baseline; keeping sample",
        );
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use anyhow::anyhow;

    use super::{new_collector, with_collector_blocking};

    #[tokio::test(flavor = "current_thread")]
    async fn collector_blocking_helper_keeps_async_runtime_responsive() {
        let mut collector = new_collector();
        let blocking = with_collector_blocking(&mut collector, |_collector| {
            std::thread::sleep(Duration::from_millis(50));
            Err::<(), _>(anyhow!("finished after blocking sleep"))
        });
        tokio::pin!(blocking);

        tokio::select! {
            _ = tokio::time::sleep(Duration::from_millis(10)) => {}
            result = &mut blocking => panic!("blocking collector finished too early: {result:?}"),
        }

        let error = blocking
            .await
            .expect_err("test operation should return its sentinel error");
        assert!(error.to_string().contains("finished after blocking sleep"));
    }
}
