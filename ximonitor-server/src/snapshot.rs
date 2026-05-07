use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use tokio::fs;
use tokio::time::interval;
use tracing::warn;
use ximonitor_proto::NodeStatus;

use crate::state::SharedState;

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
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).await.with_context(|| {
                format!("failed to create snapshot directory {}", parent.display())
            })?;
        }
    }

    let payload =
        serde_json::to_vec_pretty(statuses).context("failed to serialize node snapshot")?;
    let temporary_path = temporary_snapshot_path(path);
    fs::write(&temporary_path, payload).await.with_context(|| {
        format!(
            "failed to write temporary snapshot {}",
            temporary_path.display()
        )
    })?;
    fs::rename(&temporary_path, path)
        .await
        .with_context(|| format!("failed to move snapshot into place at {}", path.display()))?;
    Ok(())
}

fn temporary_snapshot_path(path: &Path) -> PathBuf {
    let mut temporary = path.as_os_str().to_os_string();
    temporary.push(".tmp");
    temporary.into()
}
