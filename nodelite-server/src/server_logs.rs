//! Server 运行日志的持久化存储,通过 tracing subscriber 收集后写入 SQLite。

mod query;
mod storage;
mod writer;

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use rusqlite::Connection;
use tokio::sync::{Mutex, RwLock, mpsc};
use tokio::task::JoinHandle;
use tracing::error;

use self::query::{query_logs_by_time_range, database_size_bytes, prune_by_size, ServerLogEntryWithLevel};
use self::storage::{initialize_database, open_read_connection};
use self::writer::{PendingServerLogEntry, WriterContext, run_server_log_writer};

const MAX_SERVER_LOGS_MEMORY: usize = 1000;
const SERVER_LOG_SHARDS: usize = 4;

/// Server 日志中的单条结构化条目。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerLogEntry {
    pub occurred_at: String,
    pub level: String,
    pub target: String,
    pub message: String,
}

/// Server 日志存储,支持内存缓冲 + SQLite 持久化。
pub struct ServerLogStore {
    inner: Arc<ServerLogStoreInner>,
}

struct ServerLogStoreInner {
    shards: Vec<RwLock<ServerLogShard>>,
    total_entries: AtomicUsize,
    next_sequence: AtomicU64,
    db_path: Option<Arc<PathBuf>>,
    available: Arc<AtomicBool>,
    writer_tx: Arc<RwLock<Option<mpsc::Sender<PendingServerLogEntry>>>>,
    writer_handle: Arc<Mutex<Option<JoinHandle<()>>>>,
    max_size_bytes: Option<u64>,
}

struct ServerLogShard {
    buffer: VecDeque<ServerLogEntry>,
}

impl ServerLogStore {
    /// 创建纯内存模式的 ServerLogStore。
    #[allow(dead_code)]
    pub fn new() -> Self {
        let shards = (0..SERVER_LOG_SHARDS)
            .map(|_| {
                RwLock::new(ServerLogShard {
                    buffer: VecDeque::with_capacity(MAX_SERVER_LOGS_MEMORY / SERVER_LOG_SHARDS),
                })
            })
            .collect();

        Self {
            inner: Arc::new(ServerLogStoreInner {
                shards,
                total_entries: AtomicUsize::new(0),
                next_sequence: AtomicU64::new(0),
                db_path: None,
                available: Arc::new(AtomicBool::new(true)),
                writer_tx: Arc::new(RwLock::new(None)),
                writer_handle: Arc::new(Mutex::new(None)),
                max_size_bytes: None,
            }),
        }
    }

    /// 创建持久化模式的 ServerLogStore。
    pub fn with_persistence(db_path: PathBuf, max_size_mb: u64) -> Self {
        let shards = (0..SERVER_LOG_SHARDS)
            .map(|_| {
                RwLock::new(ServerLogShard {
                    buffer: VecDeque::with_capacity(MAX_SERVER_LOGS_MEMORY / SERVER_LOG_SHARDS),
                })
            })
            .collect();

        Self {
            inner: Arc::new(ServerLogStoreInner {
                shards,
                total_entries: AtomicUsize::new(0),
                next_sequence: AtomicU64::new(0),
                db_path: Some(Arc::new(db_path)),
                available: Arc::new(AtomicBool::new(false)),
                writer_tx: Arc::new(RwLock::new(None)),
                writer_handle: Arc::new(Mutex::new(None)),
                max_size_bytes: Some(max_size_mb * 1024 * 1024),
            }),
        }
    }

    /// 初始化持久化层:创建数据库、启动 writer 任务。
    pub async fn initialize(&self) -> Result<()> {
        let Some(db_path) = self.inner.db_path.as_ref() else {
            return Ok(());
        };

        let db_path = Arc::clone(db_path);
        let connection = tokio::task::spawn_blocking(move || {
            let conn = initialize_database(&db_path)?;
            Ok::<_, anyhow::Error>(Arc::new(Mutex::new(conn)))
        })
        .await
        .context("join initialize task")??;

        let (tx, rx) = mpsc::channel(256);
        *self.inner.writer_tx.write().await = Some(tx);

        let writer_ctx = WriterContext {
            db_path: Arc::clone(self.inner.db_path.as_ref().unwrap()),
            connection,
            rx,
        };

        let handle = tokio::spawn(run_server_log_writer(writer_ctx));
        *self.inner.writer_handle.lock().await = Some(handle);
        self.inner.available.store(true, Ordering::Release);

        Ok(())
    }

    /// 关闭 writer 任务并等待刷盘。
    #[allow(dead_code)]
    pub async fn shutdown(&self) -> Result<()> {
        *self.inner.writer_tx.write().await = None;
        if let Some(handle) = self.inner.writer_handle.lock().await.take() {
            handle.await.context("join server log writer task")?;
        }
        Ok(())
    }

    /// 持久化层是否可用。
    #[allow(dead_code)]
    pub fn is_available(&self) -> bool {
        self.inner.available.load(Ordering::Acquire)
    }

    /// 记录一条 server 日志到内存 + 持久化。
    pub async fn record_entry(&self, entry: ServerLogEntry) {
        let sequence = self.inner.next_sequence.fetch_add(1, Ordering::Relaxed);
        let shard_idx = (sequence as usize) % SERVER_LOG_SHARDS;

        // 写入内存
        {
            let mut shard = self.inner.shards[shard_idx].write().await;
            if shard.buffer.len() >= MAX_SERVER_LOGS_MEMORY / SERVER_LOG_SHARDS {
                shard.buffer.pop_front();
            } else {
                self.inner.total_entries.fetch_add(1, Ordering::Relaxed);
            }
            shard.buffer.push_back(entry.clone());
        }

        // 写入持久化
        if let Some(tx) = self.inner.writer_tx.read().await.as_ref() {
            let pending = PendingServerLogEntry {
                occurred_at: entry.occurred_at,
                level: entry.level,
                target: entry.target,
                message: entry.message,
                sequence,
            };
            if tx.send(pending).await.is_err() {
                error!("server log writer channel closed");
            }
        }
    }

    /// 从内存获取最近的 N 条日志。
    #[allow(dead_code)]
    pub async fn recent_logs(&self, limit: usize) -> Vec<ServerLogEntry> {
        let mut all = Vec::new();
        for shard in &self.inner.shards {
            let guard = shard.read().await;
            all.extend(guard.buffer.iter().cloned());
        }
        all.sort_by(|a, b| b.occurred_at.cmp(&a.occurred_at));
        all.truncate(limit);
        all
    }

    /// 从持久化存储查询时间范围内的日志。
    #[allow(dead_code)]
    pub async fn query_by_time_range(
        &self,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
        level: Option<String>,
        limit: usize,
    ) -> Result<Vec<ServerLogEntryWithLevel>> {
        let Some(db_path) = self.inner.db_path.as_ref() else {
            return Ok(Vec::new());
        };

        let db_path = Arc::clone(db_path);
        tokio::task::spawn_blocking(move || {
            let conn = open_read_connection(&db_path)?;
            query_logs_by_time_range(&conn, start, end, level.as_deref(), limit)
        })
        .await
        .context("join query task")?
    }

    /// 清理超过大小限制的旧日志。
    #[allow(dead_code)]
    pub async fn prune_if_needed(&self) -> Result<usize> {
        let Some(db_path) = self.inner.db_path.as_ref() else {
            return Ok(0);
        };
        let Some(max_size_bytes) = self.inner.max_size_bytes else {
            return Ok(0);
        };

        let db_path = Arc::clone(db_path);
        tokio::task::spawn_blocking(move || {
            let mut conn = Connection::open(&*db_path)
                .with_context(|| format!("open server logs db for prune at {}", db_path.display()))?;
            prune_by_size(&mut conn, max_size_bytes)
        })
        .await
        .context("join prune task")?
    }
}
