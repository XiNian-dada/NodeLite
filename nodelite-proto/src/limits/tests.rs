//! 共享上限必须把多挂载点快照裁剪到两端一致的条目数与帧大小内。

use super::*;
use crate::{DiskUsage, MetricsMessage, WireMessage};

fn snapshot_with_disks(count: usize, mount_len: usize) -> NodeSnapshot {
    let mut snapshot: NodeSnapshot = serde_json::from_value(serde_json::json!({
        "collected_at": "2026-09-14T00:00:00Z", "cpu_usage_percent": 12.5,
        "load": {"one": 0.1, "five": 0.2, "fifteen": 0.3},
        "memory": {"total_bytes": 1024, "used_bytes": 512, "available_bytes": 512, "swap_total_bytes": 0, "swap_used_bytes": 0},
        "uptime_secs": 42, "disks": [],
        "network": {"total_rx_bytes": 123, "total_tx_bytes": 456, "rx_bytes_per_sec": null, "tx_bytes_per_sec": null, "packet_loss_percent": null}
    }))
    .expect("sample snapshot");
    snapshot.disks = (0..count)
        .map(|index| DiskUsage {
            device: format!("rpool/data/subvol-{index}"),
            mount_point: format!("/{index:04}{}", "m".repeat(mount_len)),
            fs_type: "zfs".to_string(),
            total_bytes: 100 + index as u64,
            available_bytes: 40,
            used_bytes: 60,
            used_percent: 60.0,
        })
        .collect();
    snapshot
}

fn wire_len(snapshot: NodeSnapshot) -> usize {
    serde_json::to_vec(&WireMessage::Metrics(MetricsMessage { snapshot }))
        .expect("serialize wire message")
        .len()
}

#[test]
fn shared_limits_back_server_defaults() {
    assert_eq!(
        crate::config::DEFAULT_MAX_SANITIZED_DISKS,
        MAX_SNAPSHOT_DISKS
    );
    assert_eq!(
        crate::config::DEFAULT_MAX_MESSAGE_BYTES,
        MAX_METRICS_FRAME_BYTES
    );
    assert_eq!(
        crate::config::DEFAULT_MAX_SANITIZED_STRING_BYTES,
        MAX_SNAPSHOT_STRING_BYTES
    );
}

#[test]
fn trims_excess_disks_to_shared_count_limit() {
    let mut snapshot = snapshot_with_disks(MAX_SNAPSHOT_DISKS + 4, 8);
    let dropped = fit_snapshot_to_wire_limits(&mut snapshot).expect("fit snapshot");
    assert_eq!(dropped, 4);
    assert_eq!(snapshot.disks.len(), MAX_SNAPSHOT_DISKS);
    assert!(snapshot.disks[0].mount_point.starts_with("/0000"));
}

#[test]
fn trims_long_disk_entries_to_shared_frame_limit() {
    let mut snapshot = snapshot_with_disks(MAX_SNAPSHOT_DISKS, 8);
    // 字符串已限长，只有 JSON 转义膨胀才可能让 64 条磁盘超过帧上限。
    let escaped = "\"".repeat(MAX_SNAPSHOT_STRING_BYTES);
    for (index, disk) in snapshot.disks.iter_mut().enumerate() {
        disk.device = escaped.clone();
        disk.fs_type = escaped.clone();
        disk.mount_point = format!("/{index:04}{escaped}");
    }
    assert!(wire_len(snapshot.clone()) > MAX_METRICS_FRAME_BYTES);
    let dropped = fit_snapshot_to_wire_limits(&mut snapshot).expect("fit snapshot");
    assert!(dropped > 0);
    assert!(!snapshot.disks.is_empty());
    assert!(wire_len(snapshot) <= MAX_METRICS_FRAME_BYTES);
}

#[test]
fn leaves_small_snapshots_untouched() {
    let mut snapshot = snapshot_with_disks(3, 8);
    assert_eq!(
        fit_snapshot_to_wire_limits(&mut snapshot).expect("fit snapshot"),
        0
    );
    assert_eq!(snapshot.disks.len(), 3);
}

#[test]
fn truncates_long_mount_points_at_a_char_boundary() {
    let mut snapshot = snapshot_with_disks(1, 0);
    // 奇数前缀让多字节字符跨过上限，验证不会切开 UTF-8 序列。
    snapshot.disks[0].mount_point = format!("/{}", "目".repeat(MAX_SNAPSHOT_STRING_BYTES));
    assert_eq!(
        fit_snapshot_to_wire_limits(&mut snapshot).expect("fit snapshot"),
        0
    );
    let mount_point = &snapshot.disks[0].mount_point;
    assert!(mount_point.len() <= MAX_SNAPSHOT_STRING_BYTES);
    assert!(mount_point.len() > MAX_SNAPSHOT_STRING_BYTES - 3);
    assert!(mount_point.ends_with('目'));
}

#[test]
fn drops_mount_points_that_collide_after_truncation() {
    let mut snapshot = snapshot_with_disks(3, 0);
    let shared = "m".repeat(MAX_SNAPSHOT_STRING_BYTES);
    snapshot.disks[0].mount_point = format!("{shared}/first");
    snapshot.disks[1].mount_point = format!("{shared}/second");
    assert_eq!(
        fit_snapshot_to_wire_limits(&mut snapshot).expect("fit snapshot"),
        1
    );
    assert_eq!(snapshot.disks.len(), 2);
    assert_eq!(snapshot.disks[0].device, "rpool/data/subvol-0");
    assert_eq!(snapshot.disks[0].mount_point, shared);
    assert_eq!(snapshot.disks[1].device, "rpool/data/subvol-2");
}

#[test]
fn truncates_long_device_and_fs_type_at_a_char_boundary() {
    let mut snapshot = snapshot_with_disks(1, 0);
    let long = format!("/{}", "目".repeat(MAX_SNAPSHOT_STRING_BYTES));
    snapshot.disks[0].device = long.clone();
    snapshot.disks[0].fs_type = long;
    assert_eq!(
        fit_snapshot_to_wire_limits(&mut snapshot).expect("fit snapshot"),
        0
    );
    for value in [&snapshot.disks[0].device, &snapshot.disks[0].fs_type] {
        assert!(value.len() <= MAX_SNAPSHOT_STRING_BYTES);
        assert!(value.ends_with('目'));
    }
}
