use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use tokio::fs;
use tokio::time::interval;
use tracing::warn;
use ximonitor_proto::NodeStatus;

use crate::state::SharedState;

#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

pub async fn load_snapshot(path: &Path) -> Result<Vec<NodeStatus>> {
    let content = fs::read_to_string(path)
        .await
        .with_context(|| format!("failed to read snapshot file {}", path.display()))?;
    let statuses = serde_json::from_str::<Vec<NodeStatus>>(&content)
        .with_context(|| format!("failed to parse snapshot file {}", path.display()))?;
    Ok(statuses)
}

pub fn spawn_snapshot_persistor(shared: SharedState, snapshot_path: PathBuf) {
    let snapshot_path = Arc::new(snapshot_path);
    tokio::spawn(async move {
        let mut ticker = interval(Duration::from_secs(15));
        loop {
            ticker.tick().await;
            let statuses = shared.list_statuses().await;
            if let Err(error) = persist_snapshot(snapshot_path.as_ref(), &statuses).await {
                warn!(error = ?error, path = %snapshot_path.display(), "failed to persist node snapshot");
            }
        }
    });
}

async fn persist_snapshot(path: &Path, statuses: &[NodeStatus]) -> Result<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)
            .await
            .with_context(|| format!("failed to create snapshot directory {}", parent.display()))?;
    }

    let payload =
        serde_json::to_vec_pretty(statuses).context("failed to serialize node snapshot")?;
    let temporary_path = temporary_snapshot_path(path);
    let temporary_path_for_write = temporary_path.clone();
    tokio::task::spawn_blocking(move || {
        write_snapshot_payload(&temporary_path_for_write, &payload)
    })
    .await
    .context("snapshot write task failed")??;
    fs::rename(&temporary_path, path)
        .await
        .with_context(|| format!("failed to move snapshot into place at {}", path.display()))?;
    harden_snapshot_permissions(path)?;
    Ok(())
}

fn temporary_snapshot_path(path: &Path) -> PathBuf {
    let mut temporary = path.as_os_str().to_os_string();
    temporary.push(".tmp");
    temporary.into()
}

fn write_snapshot_payload(path: &Path, payload: &[u8]) -> Result<()> {
    let mut options = std::fs::OpenOptions::new();
    options.create(true).truncate(true).write(true);
    #[cfg(unix)]
    options.mode(0o600);

    let mut file = options
        .open(path)
        .with_context(|| format!("failed to open {}", path.display()))?;
    use std::io::Write;
    file.write_all(payload)
        .with_context(|| format!("failed to write {}", path.display()))?;
    harden_snapshot_permissions(path)?;
    Ok(())
}

fn harden_snapshot_permissions(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
            .with_context(|| format!("failed to chmod {}", path.display()))?;
    }

    #[cfg(not(unix))]
    {
        let _ = path;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use chrono::Utc;
    use tokio::runtime::Runtime;
    use ximonitor_proto::{NodeIdentity, NodeStatus};

    use super::persist_snapshot;

    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

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
                std::env::temp_dir().join(format!("ximonitor-snapshot-mode-test-{unique}"));
            std::fs::create_dir_all(&temp_dir).expect("temp dir should exist");
            let snapshot_path = temp_dir.join("snapshot.json");
            let statuses = vec![NodeStatus {
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
                snapshot: None,
                last_seen: Some(Utc::now()),
                latency_ms: None,
                online: false,
            }];

            persist_snapshot(&snapshot_path, &statuses)
                .await
                .expect("snapshot should persist");

            let mode = std::fs::metadata(&snapshot_path)
                .expect("snapshot metadata")
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(mode, 0o600);

            let _ = std::fs::remove_file(&snapshot_path);
            let _ = std::fs::remove_dir(&temp_dir);
        });
    }
}
