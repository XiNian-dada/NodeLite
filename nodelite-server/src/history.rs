//! 历史数据存储:把 `NodeStatus` 中的关键指标写入本地 SQLite 表,
//! 供前端绘制趋势图。设计目标:
//!
//! - 不阻塞实时 WebSocket 流程:`record_status` 只做"节流检查 + push 到 channel",
//!   真正的 SQLite I/O 在独立的 writer task 中以 batch transaction 形式执行;
//! - 节流:同一节点两次写入至少间隔 `DEFAULT_HISTORY_WRITE_INTERVAL_SECS` 秒;
//! - 自清理:每 5 分钟最多触发一次 `DELETE`,把超过保留期的旧记录删除;
//! - 自降级:数据库初始化失败时不阻断服务,而是把 `available=false`,实时视图照常运行;
//! - 背压:bounded channel + try_send,溢出时记日志并自增 dropped 计数,
//!   而不是反压实时心跳路径(实时数据比历史趋势更重要)。
//! - 对外暴露的查询入口统一返回 [`HistoryError`],让 handler 可以按类别区分
//!   "任务调度失败" / "连接未初始化" / "查询本身失败",而不是匹配错误字符串。

mod init;
mod query;
mod writer;

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Context, anyhow};
use chrono::{DateTime, Utc};
use lru::LruCache;
use nodelite_proto::{
    DEFAULT_HISTORY_RETENTION_HOURS, DEFAULT_HISTORY_WRITE_INTERVAL_SECS, HistoryPoint, NodeStatus,
};
use parking_lot::Mutex as ParkingLotMutex;
use rusqlite::Connection;
use tokio::sync::{Mutex, RwLock, mpsc};
use tokio::task::JoinHandle;
use tracing::{error, warn};

use self::init::{initialize_database, open_read_connection};
#[cfg(test)]
use self::query::HISTORY_QUERY_SQL;
use self::query::{HistoryQueryError, query_history_between};
use self::writer::{WriterContext, build_history_point, run_history_writer};
#[cfg(test)]
use self::writer::{sqlite_busy_retry_delay, write_history_point};
use crate::queue::{QueueSendError, bounded_mpsc_channel, record_dropped_write, try_enqueue};

/// SQLite 在并发写入冲突时的等待时长。
const SQLITE_BUSY_MAX_RETRIES: u32 = 10;
const SQLITE_BUSY_RETRY_BASE_MS: u64 = 50;
const SQLITE_BUSY_RETRY_MAX_MS: u64 = 1_000;

/// Writer 任务的 channel 容量。1000 节点 × 1Hz 上报理论峰值是 1000 msg/s,
/// 在 100ms flush 间隔下,稳态 backlog ≤ ~100。1024 留出 10x 余量
/// (突发重连 / Burst 上报),仍然不至于让 record_status 真的被反压。
const HISTORY_CHANNEL_CAPACITY: usize = 1024;
/// 一次事务里最多打包多少条 INSERT。给上限是为了在极端 burst 下
/// 防止单个 fsync 周期被一直推迟。
const HISTORY_BATCH_MAX: usize = 128;
/// 没有新写入到达时,writer 也每隔这段时间 flush 一次当前 batch,
/// 保证最后几条样本不会无限期停留在内存。
const HISTORY_BATCH_FLUSH_INTERVAL: Duration = Duration::from_millis(100);
/// 距上一次 DELETE 至少要过这么长时间才再次触发清理。
const HISTORY_PRUNE_MIN_INTERVAL: Duration = Duration::from_secs(300);
/// 查询缓存容量:缓存最近 N 次查询结果。假设 50 个节点被高频查看,每节点最多缓存 4 次不同窗口查询。
const HISTORY_CACHE_CAPACITY: usize = 200;
/// 缓存条目有效期:1 秒内的重复查询直接返回缓存。
/// 短 TTL 确保快速写入场景（如测试 seed）能及时看到新数据,
/// 同时仍能为真实高频刷新（浏览器每秒轮询）提供缓存收益。
const HISTORY_CACHE_TTL: Duration = Duration::from_secs(1);

pub type HistoryResult<T> = std::result::Result<T, HistoryError>;

/// 历史查询路径对外暴露的稳定错误边界。
#[derive(Debug)]
pub enum HistoryError {
    Query(anyhow::Error),
    TaskFailed(anyhow::Error),
}

/// 查询缓存的键:(node_id, since_ts, until_ts, max_points)。
type CacheKey = (String, i64, i64, usize);

/// 查询缓存的值:(查询结果, 缓存时间戳)。
struct CacheEntry {
    points: Vec<HistoryPoint>,
    cached_at: Instant,
}

impl std::fmt::Display for HistoryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Query(_) => f.write_str("history query failed"),
            Self::TaskFailed(_) => f.write_str("history query task failed"),
        }
    }
}

impl std::error::Error for HistoryError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Query(error) | Self::TaskFailed(error) => Some(error.root_cause()),
        }
    }
}

impl From<HistoryQueryError> for HistoryError {
    fn from(error: HistoryQueryError) -> Self {
        Self::Query(anyhow!(error))
    }
}

/// 对外暴露的历史存储句柄,可被低成本克隆给多个异步任务。
#[derive(Clone)]
pub struct HistoryStore {
    db_path: Arc<PathBuf>,
    available: Arc<AtomicBool>,
    /// 持久化 SQLite 写连接,由 writer task 短暂持有。
    write_connection: Arc<Mutex<Option<Connection>>>,
    /// 节点 → 上一次成功节流通过的时间。每次 record_status 只做一次
    /// lock 往返(check + enqueue + 乐观更新),guard 不跨越任何 await/spawn_blocking,
    /// 因此与 SQLite I/O 解耦。
    last_written_at: Arc<Mutex<HashMap<String, DateTime<Utc>>>>,
    /// 最近一次执行清理删除的 Unix 时间戳(0 = 从未)。Writer task 内部更新,
    /// 用 AtomicI64 避免再多一把 mutex。
    last_pruned_at: Arc<AtomicI64>,
    /// WAL/SHM 文件通常在第一次真实写入后才出现,此标志确保后续权限加固只做一次。
    artifacts_hardened_after_write: Arc<AtomicBool>,
    /// SQLite 忙等待超时(秒)。
    sqlite_busy_timeout_secs: u64,
    /// Writer task 的入口。Some 表示已初始化;关停时清空以让 record_status 进入空操作分支。
    writer_tx: Arc<RwLock<Option<mpsc::Sender<HistoryPoint>>>>,
    /// Writer task 的 join handle,用于在关停时显式 await。
    writer_handle: Arc<Mutex<Option<JoinHandle<()>>>>,
    /// 历史写入被静默丢弃的总数(channel 满或已关闭)。监控该值可观察反压。
    dropped_writes: Arc<AtomicU64>,
    /// 查询结果 LRU 缓存,减少重复聚合的开销。使用 parking_lot::Mutex 降低锁竞争。
    query_cache: Arc<ParkingLotMutex<LruCache<CacheKey, CacheEntry>>>,
}

impl HistoryStore {
    pub fn new(db_path: PathBuf, sqlite_busy_timeout_secs: u64) -> Self {
        Self {
            db_path: Arc::new(db_path),
            available: Arc::new(AtomicBool::new(false)),
            write_connection: Arc::new(Mutex::new(None)),
            last_written_at: Arc::new(Mutex::new(HashMap::new())),
            last_pruned_at: Arc::new(AtomicI64::new(0)),
            artifacts_hardened_after_write: Arc::new(AtomicBool::new(false)),
            sqlite_busy_timeout_secs,
            writer_tx: Arc::new(RwLock::new(None)),
            writer_handle: Arc::new(Mutex::new(None)),
            dropped_writes: Arc::new(AtomicU64::new(0)),
            query_cache: Arc::new(ParkingLotMutex::new(LruCache::new(
                std::num::NonZeroUsize::new(HISTORY_CACHE_CAPACITY).expect("cache capacity > 0"),
            ))),
        }
    }

    /// 初始化数据库 + 启动 writer task。失败不会抛出,仅记录警告并保持 `available=false`。
    pub async fn initialize(&self) {
        let db_path = Arc::clone(&self.db_path);
        let sqlite_busy_timeout_secs = self.sqlite_busy_timeout_secs;
        let result = tokio::task::spawn_blocking(move || {
            let write_connection = initialize_database(db_path.as_ref(), sqlite_busy_timeout_secs)?;
            anyhow::Ok(write_connection)
        })
        .await
        .context("history database task failed");

        match result {
            Ok(Ok(write_connection)) => {
                {
                    let mut guard = self.write_connection.lock().await;
                    *guard = Some(write_connection);
                }
                self.available.store(true, Ordering::Relaxed);
                self.spawn_writer_task().await;
            }
            Ok(Err(error)) => {
                error!(
                    error = ?error,
                    "history database unavailable; real-time views will continue"
                );
            }
            Err(error) => {
                error!(error = ?error, "history database initialization join failed");
            }
        }
    }

    /// 启动后台 writer task:消费 channel,做 batch transaction 写入。
    async fn spawn_writer_task(&self) {
        let (tx, rx) = bounded_mpsc_channel(HISTORY_CHANNEL_CAPACITY);
        {
            let mut guard = self.writer_tx.write().await;
            *guard = Some(tx);
        }
        let context = WriterContext {
            db_path: Arc::clone(&self.db_path),
            write_connection: Arc::clone(&self.write_connection),
            last_pruned_at: Arc::clone(&self.last_pruned_at),
            artifacts_hardened_after_write: Arc::clone(&self.artifacts_hardened_after_write),
        };
        let handle = tokio::spawn(run_history_writer(rx, context));
        let mut guard = self.writer_handle.lock().await;
        *guard = Some(handle);
    }

    pub fn is_available(&self) -> bool {
        self.available.load(Ordering::Relaxed)
    }

    /// 统计有多少次 record_status 因为 channel 满而被静默丢弃。
    /// 监控接入后,这个计数应当长期保持在 0;持续非零表示 batch 速率跟不上上报速率。
    pub fn dropped_writes(&self) -> u64 {
        self.dropped_writes.load(Ordering::Relaxed)
    }

    pub(crate) async fn writer_queue_metrics(&self) -> (u64, u64) {
        let guard = self.writer_tx.read().await;
        let Some(tx) = guard.as_ref() else {
            return (0, 0);
        };
        let capacity = tx.max_capacity();
        let depth = capacity.saturating_sub(tx.capacity());
        (depth as u64, capacity as u64)
    }

    /// 尝试把一次节点状态记录到历史表。
    ///
    /// 节流通过 + channel 有空闲槽位时立即返回;否则静默 drop 并自增计数。
    /// 调用方(WebSocket 心跳路径)永远不会因为 SQLite I/O 而被阻塞。
    pub async fn record_status(&self, status: &NodeStatus) {
        self.record_status_with_builder(status, build_history_point)
            .await;
    }

    async fn record_status_with_builder<F>(&self, status: &NodeStatus, build_point: F)
    where
        F: FnOnce(&NodeStatus) -> Option<HistoryPoint>,
    {
        if !self.is_available() {
            return;
        }

        // 先取 writer sender 再进节流 mutex,避免持有 mutex 时再 await 另一把锁。
        // writer_tx 的写者只有初始化/关停两处,这次读锁几乎无竞争。
        let tx = {
            let guard = self.writer_tx.read().await;
            guard.as_ref().cloned()
        };
        let Some(tx) = tx else {
            return;
        };

        // 单次 mutex 往返完成"节流检查 + enqueue + 更新水位"。
        // 检查在 build_point 之前,被节流的心跳(常态路径)不付出构造样本的成本;
        // guard 跨越的全部是同步操作,不会把锁持有期拖进任何 await。
        let recorded_at = status.last_seen.unwrap_or_else(Utc::now);
        let mut throttle = self.last_written_at.lock().await;
        if let Some(previous) = throttle.get(status.identity.node_id.as_str()) {
            let Ok(elapsed) = recorded_at
                .signed_duration_since(previous.to_owned())
                .to_std()
            else {
                return;
            };
            if elapsed < Duration::from_secs(DEFAULT_HISTORY_WRITE_INTERVAL_SECS) {
                return;
            }
        }

        let Some(point) = build_point(status) else {
            return;
        };
        let node_id = point.node_id.clone();
        let recorded_at = point.recorded_at;

        // try_send 在 channel 满时立即失败,这里宁可丢一条样本
        // 也不要让 WS 处理路径被反压;丢弃由 dropped_writes 计数提示运维。
        match try_enqueue(&tx, point) {
            Ok(()) => {
                throttle.insert(node_id, recorded_at);
            }
            Err(QueueSendError::Full) => {
                let dropped_total = record_dropped_write(&self.dropped_writes);
                warn!(
                    capacity = HISTORY_CHANNEL_CAPACITY,
                    dropped_total, "history writer queue full; dropping write"
                );
            }
            Err(QueueSendError::Closed) => {
                // Writer 已退出 — 标记 store 整体不可用,后续 record_status 会快速 return。
                self.available.store(false, Ordering::Relaxed);
            }
        }
    }

    /// 关停:drop sender → writer task 自动 drain 残留 batch 后退出,
    /// 然后 await 它结束。在 `run_server` 的 shutdown 阶段调用,
    /// 保证 SIGTERM 时已经被 enqueue 的样本不会随进程退出而丢失。
    pub async fn shutdown(&self) {
        let sender = {
            let mut guard = self.writer_tx.write().await;
            guard.take()
        };
        drop(sender);

        let handle = {
            let mut guard = self.writer_handle.lock().await;
            guard.take()
        };
        if let Some(handle) = handle
            && let Err(error) = handle.await
        {
            warn!(error = ?error, "history writer task join failed during shutdown");
        }
    }

    /// 按"过去 N 小时"窗口查询历史记录。
    pub async fn query_history(
        &self,
        node_id: &str,
        window_hours: u64,
        max_points: usize,
    ) -> HistoryResult<Vec<HistoryPoint>> {
        if !self.is_available() {
            return Ok(Vec::new());
        }

        let clamped_window_hours = window_hours.clamp(1, DEFAULT_HISTORY_RETENTION_HOURS);
        let clamped_max_points = max_points.max(60);
        let since = Utc::now() - chrono::Duration::hours(clamped_window_hours as i64);
        let until = Utc::now();

        self.query_history_with_cache(node_id, since, until, clamped_max_points)
            .await
    }

    /// 按"任意时间区间"查询历史记录。超出保留期或反向区间会被自动裁剪。
    pub async fn query_history_range(
        &self,
        node_id: &str,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
        max_points: usize,
    ) -> HistoryResult<Vec<HistoryPoint>> {
        if !self.is_available() {
            return Ok(Vec::new());
        }

        let now = Utc::now();
        let retention_floor = now - chrono::Duration::hours(DEFAULT_HISTORY_RETENTION_HOURS as i64);
        let clamped_start = start.max(retention_floor);
        let clamped_end = end.min(now);
        if clamped_end <= clamped_start {
            return Ok(Vec::new());
        }

        let clamped_max_points = max_points.max(60);

        self.query_history_with_cache(node_id, clamped_start, clamped_end, clamped_max_points)
            .await
    }

    /// 带缓存的查询逻辑:检查缓存 → 缓存命中直接返回 → 缓存未命中执行查询并写入缓存。
    async fn query_history_with_cache(
        &self,
        node_id: &str,
        since: DateTime<Utc>,
        until: DateTime<Utc>,
        max_points: usize,
    ) -> HistoryResult<Vec<HistoryPoint>> {
        let cache_key = (
            node_id.to_string(),
            since.timestamp(),
            until.timestamp(),
            max_points,
        );

        // 先尝试从缓存读取（parking_lot::Mutex 是同步的，不需要 await）
        {
            let mut cache = self.query_cache.lock();
            if let Some(entry) = cache.get(&cache_key) {
                let age = entry.cached_at.elapsed();
                if age < HISTORY_CACHE_TTL {
                    return Ok(entry.points.clone());
                }
                // TTL 过期,移除旧条目
                cache.pop(&cache_key);
            }
        }

        // 缓存未命中或已过期,执行实际查询
        let db_path = Arc::clone(&self.db_path);
        let node_id = node_id.to_string();
        let sqlite_busy_timeout_secs = self.sqlite_busy_timeout_secs;

        let points = tokio::task::spawn_blocking(move || {
            let connection = open_read_connection(db_path.as_ref(), sqlite_busy_timeout_secs)
                .map_err(HistoryError::Query)?;
            query_history_between(
                &connection,
                &node_id,
                since,
                until,
                max_points,
            )
            .map_err(HistoryError::from)
        })
        .await
        .map_err(|error| {
            HistoryError::TaskFailed(anyhow!("history range query task failed: {error}"))
        })??;

        // 写入缓存（只缓存非空结果，避免测试场景中缓存"数据还没写入"的空快照）
        if !points.is_empty() {
            let mut cache = self.query_cache.lock();
            cache.put(
                cache_key,
                CacheEntry {
                    points: points.clone(),
                    cached_at: Instant::now(),
                },
            );
        }

        Ok(points)
    }

    /// 清理已经不在注册表中的节点节流状态,避免长期运行时条目只增不减。
    pub async fn forget_missing(&self, live_node_ids: &[String]) -> usize {
        let live_node_ids: HashSet<&str> = live_node_ids.iter().map(String::as_str).collect();
        let mut guard = self.last_written_at.lock().await;
        let before = guard.len();
        guard.retain(|node_id, _| live_node_ids.contains(node_id.as_str()));
        before.saturating_sub(guard.len())
    }
}

#[cfg(test)]
mod tests;
