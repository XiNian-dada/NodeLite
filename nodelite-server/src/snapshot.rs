//! 节点状态磁盘快照:为了在 Server 重启后能立即展示"上一秒"的视图,
//! 这里周期性地把 `SharedState` 的所有 `NodeStatus` 写入磁盘文件。
//!
//! Shared-state writers take the same lock before reading the latest view, so an
//! older periodic write cannot overwrite a completed node deletion.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use nodelite_proto::{GeoIpLocation, NodeStatus};
use tokio::fs;
use tokio::task::JoinHandle;
use tokio::time::{MissedTickBehavior, interval};
use tokio_util::sync::CancellationToken;
use tracing::warn;

use crate::fs_security::{PrivateWriteError, atomic_write_private, create_private_dir_all_async};
use crate::state::SharedState;

#[derive(Debug, Default)]
pub(crate) struct SnapshotPersistCheckpoint {
    last_revision: Option<u64>,
}

/// 从磁盘读取上一次的快照文件并反序列化。
pub async fn load_snapshot(path: &Path) -> Result<Vec<NodeStatus>> {
    let content = fs::read_to_string(path)
        .await
        .with_context(|| format!("failed to read snapshot file {}", path.display()))?;
    let statuses = serde_json::from_str::<Vec<NodeStatus>>(&content)
        .with_context(|| format!("failed to parse snapshot file {}", path.display()))?;
    for status in &statuses {
        crate::registry::validate_runtime_identity(&status.identity).with_context(|| {
            format!(
                "snapshot contains invalid identity for {}",
                status.identity.node_id
            )
        })?;
        validate_snapshot_location(
            "geoip",
            status.geoip_country.as_deref(),
            status.geoip_city.as_deref(),
            status.geoip_latitude,
            status.geoip_longitude,
        )?;
        validate_snapshot_location(
            "location override",
            status.location_override_country.as_deref(),
            status.location_override_city.as_deref(),
            status.location_override_latitude,
            status.location_override_longitude,
        )?;
    }
    Ok(statuses)
}

fn validate_snapshot_location(
    field: &str,
    country: Option<&str>,
    city: Option<&str>,
    latitude: Option<f64>,
    longitude: Option<f64>,
) -> Result<()> {
    if country.is_none() && city.is_none() && latitude.is_none() && longitude.is_none() {
        return Ok(());
    }
    let Some(country) = country else {
        anyhow::bail!("snapshot {field} is missing country");
    };
    crate::sanitize::validate_location_override(&GeoIpLocation {
        country: country.to_string(),
        city: city.map(str::to_string),
        latitude,
        longitude,
    })
    .map_err(|error| anyhow::anyhow!("snapshot {field} is invalid: {error}"))
}

/// 启动一个后台任务,每 15 秒把当前 `SharedState` 序列化到 `snapshot_path`。
pub fn spawn_snapshot_persistor(
    shared: SharedState,
    snapshot_path: PathBuf,
    shutdown: CancellationToken,
) -> JoinHandle<()> {
    let snapshot_path = Arc::new(snapshot_path);
    tokio::spawn(async move {
        let mut ticker = interval(Duration::from_secs(15));
        let mut checkpoint = SnapshotPersistCheckpoint::default();
        // 主机/进程被挂起恢复后,不要连续 burst 多次磁盘 IO;保持 15 s 节奏即可。
        ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                _ = shutdown.cancelled() => break,
                _ = ticker.tick() => {
                    if let Err(error) = persist_snapshot_if_changed(
                        &shared,
                        snapshot_path.as_ref(),
                        &mut checkpoint,
                    )
                    .await {
                        warn!(error = ?error, path = %snapshot_path.display(), "failed to persist node snapshot");
                    }
                }
            }
        }
    })
}

pub(crate) async fn persist_snapshot_if_changed(
    shared: &SharedState,
    path: &Path,
    checkpoint: &mut SnapshotPersistCheckpoint,
) -> Result<bool> {
    let revision = shared.nodes_revision();
    if checkpoint.last_revision == Some(revision) {
        return Ok(false);
    }

    checkpoint.last_revision = Some(persist_current_snapshot(shared, path).await?);
    Ok(true)
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum SnapshotPersistError {
    #[error("failed to prepare private snapshot directory: {0}")]
    Directory(#[source] anyhow::Error),
    #[error("failed to serialize snapshot: {0}")]
    Serialize(#[from] serde_json::Error),
    #[error("failed to commit snapshot: {0}")]
    Write(#[from] PrivateWriteError),
    #[error("snapshot persistence task failed: {0}")]
    Task(#[from] tokio::task::JoinError),
}

/// The lock covers reading the current view and committing it, including caller cancellation.
pub(crate) async fn persist_current_snapshot(
    shared: &SharedState,
    path: &Path,
) -> std::result::Result<u64, SnapshotPersistError> {
    let shared = shared.clone();
    let path = path.to_path_buf();
    tokio::spawn(async move {
        let _guard = shared.snapshot_write_lock.lock().await;
        let revision = shared.nodes_revision();
        let statuses = shared.list_statuses().await;
        persist_snapshot(&path, &statuses).await?;
        Ok(revision)
    })
    .await?
}

pub(crate) async fn persist_snapshot(
    path: &Path,
    statuses: &[NodeStatus],
) -> std::result::Result<(), SnapshotPersistError> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        create_private_dir_all_async(parent)
            .await
            .map_err(SnapshotPersistError::Directory)?;
    }
    let payload = serde_json::to_vec(statuses)?;
    let path = path.to_path_buf();
    tokio::task::spawn_blocking(move || atomic_write_private(&path, &payload)).await??;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
    use std::sync::Arc;
    use std::time::{SystemTime, UNIX_EPOCH};

    use chrono::Utc;
    use nodelite_proto::{NodeIdentity, NodeStatus};
    use tokio::runtime::Runtime;

    use super::{
        SnapshotPersistCheckpoint, load_snapshot, persist_snapshot, persist_snapshot_if_changed,
    };
    use crate::state::SharedState;
    use crate::test_support::{synthetic_identity, test_server_config};

    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    #[tokio::test]
    async fn snapshot_persistor_skips_unchanged_revision_and_writes_after_change() {
        let unique = unique_suffix();
        let temp_dir = std::env::temp_dir().join(format!("nodelite-snapshot-skip-test-{unique}"));
        std::fs::create_dir_all(&temp_dir).expect("temp dir should exist");
        let snapshot_path = temp_dir.join("snapshot.json");
        let config = test_server_config(
            SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0)),
            "http://127.0.0.1:0".to_string(),
            temp_dir.join("server.json"),
            temp_dir.join("history.sqlite3"),
            snapshot_path.clone(),
        );
        let shared = SharedState::new(Arc::new(config));
        let mut checkpoint = SnapshotPersistCheckpoint::default();

        assert!(
            persist_snapshot_if_changed(&shared, &snapshot_path, &mut checkpoint)
                .await
                .expect("initial snapshot should persist")
        );
        let initial_payload =
            std::fs::read_to_string(&snapshot_path).expect("snapshot should be readable");

        assert!(
            !persist_snapshot_if_changed(&shared, &snapshot_path, &mut checkpoint)
                .await
                .expect("unchanged snapshot should skip")
        );
        assert_eq!(
            std::fs::read_to_string(&snapshot_path).expect("snapshot should be readable"),
            initial_payload
        );

        shared
            .register_node(
                synthetic_identity("hk-01", "Hong Kong 01", "1.0.0", None, "edge"),
                Some("198.51.100.24".to_string()),
                None,
                None,
            )
            .await;

        assert!(
            persist_snapshot_if_changed(&shared, &snapshot_path, &mut checkpoint)
                .await
                .expect("changed snapshot should persist")
        );
        let restored = load_snapshot(&snapshot_path)
            .await
            .expect("snapshot should restore");
        assert_eq!(restored.len(), 1);
        assert_eq!(restored[0].identity.node_id, "hk-01");

        let _ = std::fs::remove_file(&snapshot_path);
        let _ = std::fs::remove_dir(&temp_dir);
    }

    #[tokio::test]
    async fn persisted_snapshot_uses_compact_json_and_still_restores() {
        let unique = unique_suffix();
        let temp_dir =
            std::env::temp_dir().join(format!("nodelite-snapshot-compact-test-{unique}"));
        std::fs::create_dir_all(&temp_dir).expect("temp dir should exist");
        let snapshot_path = temp_dir.join("snapshot.json");
        let statuses = vec![sample_status()];

        persist_snapshot(&snapshot_path, &statuses)
            .await
            .expect("snapshot should persist");

        let compact_payload =
            std::fs::read_to_string(&snapshot_path).expect("snapshot should be readable");
        let pretty_payload =
            serde_json::to_string_pretty(&statuses).expect("pretty json should serialize");
        assert!(
            compact_payload.len() < pretty_payload.len(),
            "compact snapshot should be smaller than pretty JSON"
        );

        let restored = load_snapshot(&snapshot_path)
            .await
            .expect("snapshot should restore");
        assert_eq!(restored.len(), 1);
        assert_eq!(restored[0].identity.node_id, "hk-01");

        let _ = std::fs::remove_file(&snapshot_path);
        let _ = std::fs::remove_dir(&temp_dir);
    }

    #[tokio::test]
    async fn load_snapshot_rejects_oversized_runtime_identity() {
        let unique = unique_suffix();
        let temp_dir =
            std::env::temp_dir().join(format!("nodelite-snapshot-identity-test-{unique}"));
        std::fs::create_dir_all(&temp_dir).expect("temp dir should exist");
        let snapshot_path = temp_dir.join("snapshot.json");
        let mut status = sample_status();
        status.identity.hostname = "界".repeat(86);

        persist_snapshot(&snapshot_path, &[status])
            .await
            .expect("snapshot should persist");
        let error = load_snapshot(&snapshot_path)
            .await
            .expect_err("oversized snapshot identity should fail");
        assert!(error.to_string().contains("invalid identity"));

        let _ = std::fs::remove_file(&snapshot_path);
        let _ = std::fs::remove_dir(&temp_dir);
    }

    #[tokio::test]
    async fn load_snapshot_rejects_oversized_geoip_text() {
        let unique = unique_suffix();
        let temp_dir = std::env::temp_dir().join(format!("nodelite-snapshot-geoip-test-{unique}"));
        std::fs::create_dir_all(&temp_dir).expect("temp dir should exist");
        let snapshot_path = temp_dir.join("snapshot.json");
        let mut status = sample_status();
        status.geoip_country = Some("HK".to_string());
        status.geoip_city = Some("x".repeat(crate::sanitize::MAX_LOCATION_OVERRIDE_TEXT_BYTES + 1));

        persist_snapshot(&snapshot_path, &[status])
            .await
            .expect("snapshot should persist");
        let error = load_snapshot(&snapshot_path)
            .await
            .expect_err("oversized snapshot GeoIP should fail");
        assert!(error.to_string().contains("snapshot geoip is invalid"));

        let _ = std::fs::remove_file(&snapshot_path);
        let _ = std::fs::remove_dir(&temp_dir);
    }

    #[test]
    #[cfg(unix)]
    fn persisted_snapshot_is_mode_600() {
        let runtime = Runtime::new().expect("runtime should build");
        runtime.block_on(async {
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock should be monotonic enough")
                .as_nanos();
            let temp_dir =
                std::env::temp_dir().join(format!("nodelite-snapshot-mode-test-{unique}"));
            std::fs::create_dir_all(&temp_dir).expect("temp dir should exist");
            let data_dir = temp_dir.join("data");
            let snapshot_path = data_dir.join("snapshot.json");
            let statuses = vec![sample_status()];

            persist_snapshot(&snapshot_path, &statuses)
                .await
                .expect("snapshot should persist");

            let dir_mode = std::fs::metadata(&data_dir)
                .expect("snapshot dir metadata")
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(dir_mode, 0o700);

            let mode = std::fs::metadata(&snapshot_path)
                .expect("snapshot metadata")
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(mode, 0o600);

            let _ = std::fs::remove_file(&snapshot_path);
            let _ = std::fs::remove_dir(&data_dir);
            let _ = std::fs::remove_dir(&temp_dir);
        });
    }

    fn sample_status() -> NodeStatus {
        NodeStatus {
            identity: NodeIdentity {
                node_id: "hk-01".to_string(),
                node_label: "Hong Kong 01".to_string(),
                hostname: "hk-01.internal".to_string(),
                os: "Ubuntu".to_string(),
                kernel_version: None,
                cpu_model: None,
                cpu_cores: 2,
                agent_version: "1.0.6".to_string(),
                boot_time: None,
                tags: vec!["edge".to_string()],
            },
            remote_ip: Some("198.51.100.24".to_string()),
            geoip_country: None,
            geoip_city: None,
            geoip_latitude: None,
            geoip_longitude: None,
            location_override_country: None,
            location_override_city: None,
            location_override_latitude: None,
            location_override_longitude: None,
            snapshot: None,
            last_seen: Some(Utc::now()),
            latency_ms: None,
            online: false,
        }
    }

    fn unique_suffix() -> u128 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be monotonic enough")
            .as_nanos()
    }
}
