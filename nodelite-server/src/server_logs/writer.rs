//! Server 日志异步批量写入器。

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use rusqlite::{Connection, params};
use tokio::sync::{mpsc, Mutex};
use tracing::{error, warn};

/// Writer 批量大小上限。
const SERVER_LOG_BATCH_MAX: usize = 128;
/// Writer flush 间隔。
const SERVER_LOG_BATCH_FLUSH_INTERVAL: Duration = Duration::from_millis(100);

/// 待写入的日志条目。
pub(super) struct PendingServerLogEntry {
    pub(super) occurred_at: String,
    pub(super) level: String,
    pub(super) target: String,
    pub(super) message: String,
    pub(super) sequence: u64,
}

/// Server 日志 writer 任务的上下文。
pub(super) struct WriterContext {
    pub(super) db_path: Arc<PathBuf>,
    pub(super) connection: Arc<Mutex<Connection>>,
    pub(super) rx: mpsc::Receiver<PendingServerLogEntry>,
}

/// 运行 server log writer 任务。
pub(super) async fn run_server_log_writer(mut ctx: WriterContext) {
    let mut batch = Vec::with_capacity(SERVER_LOG_BATCH_MAX);
    let mut flush_interval = tokio::time::interval(SERVER_LOG_BATCH_FLUSH_INTERVAL);
    flush_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            Some(entry) = ctx.rx.recv() => {
                batch.push(entry);
                if batch.len() >= SERVER_LOG_BATCH_MAX {
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

async fn flush_batch(conn: &Arc<Mutex<Connection>>, batch: &mut Vec<PendingServerLogEntry>) {
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
                "failed to write server log batch"
            );
        }
        Err(error) => {
            error!(
                count,
                error = %error,
                "server log writer task panicked"
            );
        }
    }

    batch.clear();
}

fn write_batch(conn: &mut Connection, batch: &[PendingServerLogEntry]) -> Result<()> {
    let tx = conn.transaction().context("begin server log transaction")?;
    {
        let mut stmt = tx
            .prepare_cached(
                "INSERT INTO server_logs (occurred_at, level, target, message, sequence)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
            )
            .context("prepare server log insert statement")?;

        for entry in batch {
            stmt.execute(params![
                &entry.occurred_at,
                &entry.level,
                &entry.target,
                &entry.message,
                entry.sequence as i64,
            ])
            .context("insert server log entry")?;
        }
    }
    tx.commit().context("commit server log batch")?;
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
                    "failed to harden server log WAL artifact permissions"
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
