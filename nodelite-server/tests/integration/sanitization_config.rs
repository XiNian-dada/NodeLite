//! Nondefault sanitization settings must reach real wire-message processing.

use super::*;
use crate::test_support::fake_snapshot;
use nodelite_proto::DiskUsage;

#[tokio::test]
async fn configured_disk_and_utf8_limits_apply_before_the_configured_disconnect_count() -> Result<()>
{
    for limit in [1, 3] {
        let server = TestServer::start_with_config(|config| {
            config.max_sanitized_disks = 2;
            config.max_sanitized_string_bytes = 5;
            config.metric_anomaly_session_limit = limit;
        })
        .await?;
        let node = server
            .issue_node("configured-limits", "Configured limits")
            .await?;
        let mut agent = TestAgent::connect(&server, &node).await?;
        for sequence in 1..=limit {
            let mut snapshot = fake_snapshot(sequence as u64);
            snapshot.disks = (0..4)
                .map(|index| DiskUsage {
                    device: format!("{index}设备名称"),
                    mount_point: "/目录名称".to_string(),
                    fs_type: "ext4".to_string(),
                    total_bytes: 100,
                    used_bytes: 50,
                    available_bytes: 50,
                    used_percent: 50.0,
                })
                .collect();
            agent.send_snapshot(snapshot).await?;
            if sequence < limit {
                let status = server
                    .wait_for_node_uptime(&node.node_id, sequence as u64, TEST_TIMEOUT)
                    .await?;
                let snapshot = status.snapshot.expect("sanitized snapshot");
                assert_eq!(snapshot.disks.len(), 2);
                assert_eq!(snapshot.disks[0].device, "0设");
                assert_eq!(snapshot.disks[0].mount_point, "/目");
                assert_eq!(snapshot.disks[1].device, "1设");
            }
        }
        let offline = server
            .wait_for_node_offline(&node.node_id, TEST_TIMEOUT)
            .await?;
        assert_eq!(
            offline.snapshot.map(|s| s.uptime_secs),
            (limit > 1).then_some((limit - 1) as u64)
        );
        server.shutdown().await?;
    }
    Ok(())
}

#[tokio::test]
async fn excess_valid_disks_are_trimmed_without_disconnecting_the_session() -> Result<()> {
    let server = TestServer::start_with_config(|_| {}).await?;
    let node = server.issue_node("many-mounts", "Many mounts").await?;
    let mut agent = TestAgent::connect(&server, &node).await?;
    let reports = nodelite_proto::config::DEFAULT_METRIC_ANOMALY_SESSION_LIMIT as u64 + 2;
    for sequence in 1..=reports {
        let mut snapshot = fake_snapshot(sequence);
        snapshot.disks = (0..68)
            .map(|index| DiskUsage {
                device: format!("rpool/subvol-{index}"),
                mount_point: format!("/subvol-{index}"),
                fs_type: "zfs".to_string(),
                total_bytes: 100,
                used_bytes: 50,
                available_bytes: 50,
                used_percent: 50.0,
            })
            .collect();
        agent.send_snapshot(snapshot).await?;
        let status = server
            .wait_for_node_uptime(&node.node_id, sequence, TEST_TIMEOUT)
            .await?;
        let snapshot = status.snapshot.expect("trimmed snapshot");
        assert_eq!(snapshot.disks.len(), nodelite_proto::MAX_SNAPSHOT_DISKS);
    }
    server.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn repeated_malformed_disks_still_disconnect_the_session() -> Result<()> {
    let server = TestServer::start_with_config(|config| {
        config.metric_anomaly_session_limit = 2;
    })
    .await?;
    let node = server.issue_node("malformed", "Malformed").await?;
    let mut agent = TestAgent::connect(&server, &node).await?;
    for sequence in 1..=2 {
        let mut snapshot = fake_snapshot(sequence);
        snapshot.disks = vec![DiskUsage {
            device: "   ".to_string(),
            mount_point: "/".to_string(),
            fs_type: "ext4".to_string(),
            total_bytes: 100,
            used_bytes: 50,
            available_bytes: 50,
            used_percent: 50.0,
        }];
        agent.send_snapshot(snapshot).await?;
    }
    server
        .wait_for_node_offline(&node.node_id, TEST_TIMEOUT)
        .await?;
    server.shutdown().await?;
    Ok(())
}
