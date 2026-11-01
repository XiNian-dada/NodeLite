//! 发送前把快照裁剪到与 Server 共享的上限内。
//!
//! 由 Agent 自己裁剪，Server 收到的快照就不会超出其默认清洗上限或帧上限，
//! 大挂载点主机因此不会被判为异常并反复断连。

use std::sync::atomic::{AtomicUsize, Ordering};

use anyhow::{Context, Result};
use nodelite_proto::{MAX_METRICS_FRAME_BYTES, MAX_SNAPSHOT_DISKS, NodeSnapshot};
use tracing::warn;

/// 上一次告警时的裁剪条数；只在变化时告警，避免每个上报周期刷屏。
static LAST_WARNED_DROPPED: AtomicUsize = AtomicUsize::new(0);

pub(crate) fn fit_for_wire(snapshot: &mut NodeSnapshot) -> Result<usize> {
    let collected = snapshot.disks.len();
    let dropped = nodelite_proto::fit_snapshot_to_wire_limits(snapshot)
        .context("measure metrics snapshot size")?;
    if LAST_WARNED_DROPPED.swap(dropped, Ordering::Relaxed) != dropped && dropped > 0 {
        warn!(
            collected_disks = collected,
            reported_disks = snapshot.disks.len(),
            max_disks = MAX_SNAPSHOT_DISKS,
            max_frame_bytes = MAX_METRICS_FRAME_BYTES,
            "trimming disk list to the shared snapshot limits",
        );
    }
    Ok(dropped)
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use nodelite_proto::{
        DiskUsage, LoadAverage, MAX_METRICS_FRAME_BYTES, MAX_SNAPSHOT_DISKS, MemoryUsage,
        MetricsMessage, NetworkCounters, NodeSnapshot, WireMessage,
    };

    use super::fit_for_wire;

    fn snapshot(disks: usize) -> NodeSnapshot {
        NodeSnapshot {
            collected_at: Utc::now(),
            cpu_usage_percent: Some(1.0),
            load: LoadAverage {
                one: 0.1,
                five: 0.1,
                fifteen: 0.1,
            },
            memory: MemoryUsage {
                total_bytes: 100,
                used_bytes: 50,
                available_bytes: 50,
                swap_total_bytes: 0,
                swap_used_bytes: 0,
            },
            uptime_secs: 1,
            disks: (0..disks)
                .map(|index| DiskUsage {
                    device: format!("rpool/data/subvol-{index}-disk-0"),
                    mount_point: format!("/rpool/data/subvol-{index:03}-disk-0"),
                    fs_type: "zfs".to_string(),
                    total_bytes: 1_000 + index as u64,
                    available_bytes: 400,
                    used_bytes: 600,
                    used_percent: 60.0,
                })
                .collect(),
            network: NetworkCounters {
                total_rx_bytes: 1,
                total_tx_bytes: 2,
                rx_bytes_per_sec: None,
                tx_bytes_per_sec: None,
                packet_loss_percent: None,
            },
        }
    }

    #[test]
    fn many_mount_snapshot_is_trimmed_to_shared_limits() {
        let mut snapshot = snapshot(MAX_SNAPSHOT_DISKS + 36);
        let dropped = fit_for_wire(&mut snapshot).expect("fit snapshot");
        assert_eq!(dropped, 36);
        assert_eq!(snapshot.disks.len(), MAX_SNAPSHOT_DISKS);
        let wire = serde_json::to_vec(&WireMessage::Metrics(MetricsMessage { snapshot }))
            .expect("serialize metrics");
        assert!(wire.len() <= MAX_METRICS_FRAME_BYTES);
    }

    #[test]
    fn snapshot_within_limits_is_sent_unchanged() {
        let mut snapshot = snapshot(4);
        assert_eq!(fit_for_wire(&mut snapshot).expect("fit snapshot"), 0);
        assert_eq!(snapshot.disks.len(), 4);
    }
}
