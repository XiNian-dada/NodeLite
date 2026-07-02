//! Agent 日志异步批量写入器。

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use rusqlite::{Connection, params};
use tokio::sync::{mpsc, Mutex};
use tracing::{error, warn};

/// Writer 批量大小上限。
const AGENT_LOG_BATCH_MAX: usize = 128;
/// Writer flush 间隔。
const AGENT_LOG_BATCH_FLUSH_INTERVAL: Duration = Duration::from_millis(100);

/// 待写入的日志条目。
pub(super) struct PendingLogEntry {
    pub(super) node_id: String,
    pub(super) occurred_at: String,
    pub(super) level: String,
    pub(super) message: String,
    pub(super) sequence: u64,
}

/// Agent 日志 writer 任务的上下文。
pub(super) struct WriterContext {
    pub(super) db_path: Arc<PathBuf>,
    pub(super) connection: Arc<Mutex<Connection>>,
    pub(super) rx: mpsc::Receiver<PendingLogEntry>,
}

/// 运行 agent log writer 任务。
pub(super) async fn run_agent_log_writer(mut ctx: WriterContext) {
    let mut batch = Vec::with_capacity(AGENT_LOG_BATCH_MAX);
    let mut flush_interval = tokio::time::interval(AGENT_LOG_BATCH_FLUSH_INTERVAL);
    flush_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            Some(entry) = ctx.rx.recv() => {
                batch.push(entry);
                if batch.len() >= AGENT_LOG_BATCH_MAX {
                    flush_batch(&ctx.connection, &mut batch).await;
                }
            }
            _ = flush_interval.tick() => {
                if !batch.is_empty() {
                    flush_batch(&ctx.connection, &mut batch).await;
                }
            }
            else => {
                // Channel 关闭,写入剩余 batch 后退出
                if !batch.is_empty() {
                    flush_batch(&ctx.connection, &mut batch).await;
                }
                break;
            }
        }
    }

    harden_wal_artifacts(&ctx.db_path);
}

async fn flush_batch(conn: &Arc<Mutex<Connection>>, batch: &mut Vec<PendingLogEntry>) {
    let count = batch.len();
    let batch_owned = std::mem::take(batch);
    let conn = Arc::clone(conn);

    let result = tokio::task::spawn_blocking(move || {
        let mut guard = conn.blocking_lock();
        write_batch(&mut guard, &batch_owned)
    })
    .await;

    match result {
        Ok(Ok(())) => {}
        Ok(Err(error)) => {
            warn!(
                count,
                error = %error,
                "failed to write agent log batch"
            );
        }
        Err(error) => {
            error!(
                count,
                error = %error,
                "agent log writer task panicked"
            );
        }
    }

    batch.clear();
}

fn write_batch(conn: &mut Connection, batch: &[PendingLogEntry]) -> Result<()> {
    let tx = conn.transaction().context("begin agent log transaction")?;
    {
        let mut stmt = tx
            .prepare_cached(
                "INSERT INTO agent_logs (node_id, occurred_at, level, message, sequence)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
            )
            .context("prepare agent log insert statement")?;

        for entry in batch {
            stmt.execute(params![
                &entry.node_id,
                &entry.occurred_at,
                &entry.level,
                &entry.message,
                entry.sequence as i64,
            ])
            .context("insert agent log entry")?;
        }
    }
    tx.commit().context("commit agent log batch")?;
    Ok(())
}

fn harden_wal_artifacts(db_path: &Path) {
    let wal_path = PathBuf::from(format!("{}-wal", db_path.display()));
    let shm_path = PathBuf::from(format!("{}-shm", db_path.display()));

    for artifact in [&wal_path, &shm_path] {
        if artifact.exists() {
            if let Err(error) = harden_file_permissions(artifact) {
                warn!(
                    path = %artifact.display(),
                    error = %error,
                    "failed to harden agent log WAL artifact permissions"
                );
            }
        }
    }
}

fn harden_file_permissions(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
            .with_context(|| format!("chmod 0600 {}", path.display()))?;
    }

    #[cfg(not(unix))]
    {
        let _ = path;
    }

    Ok(())
}
