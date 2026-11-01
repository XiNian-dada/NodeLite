//! 按自然月累计节点套餐流量，并把计数器状态异步写入历史 SQLite 数据库。
//!
//! Agent 的网卡累计字节会在重启时归零，因此这里保存每个节点上一次看到的
//! 收发计数与 uptime；只累加单调递增的差值，回退规则见 `counters`。计费口径或
//! 自然月变化时重建基线，避免把不同套餐的字节数混合在同一个使用量中。

mod counters;
mod health;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::{Context, Result};
use chrono::{DateTime, Datelike, TimeZone, Utc};
use parking_lot::Mutex as ParkingLotMutex;
use rusqlite::{Connection, params};
use tokio::sync::{Mutex, RwLock, mpsc, oneshot};
use tokio::task::JoinHandle;
use tokio::time::{Instant, MissedTickBehavior, interval};
use tracing::{error, info, warn};

use self::counters::CounterBaseline;
pub(crate) use self::counters::TrafficSample;
pub(crate) use self::health::TrafficLedgerHealth;
use crate::queue::bounded_mpsc_channel;
use crate::registry::TrafficAccounting;

const TRAFFIC_WRITER_CHANNEL_CAPACITY: usize = 1024;
const TRAFFIC_WRITER_FLUSH_MILLIS: u64 = 200;
const TRAFFIC_RETRY_INITIAL_DELAY: Duration = Duration::from_secs(1);
const TRAFFIC_RETRY_MAX_DELAY: Duration = Duration::from_secs(30);
pub(super) const TRAFFIC_LOAD_RETRY_INTERVAL: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TrafficUsage {
    pub(crate) node_id: String,
    pub(crate) cycle_started_at: DateTime<Utc>,
    pub(crate) used_bytes: u64,
    pub(crate) accounting: TrafficAccounting,
}

#[derive(Debug, Clone)]
struct TrafficUsageState {
    usage: TrafficUsage,
    baseline: CounterBaseline,
}

#[derive(Debug, Clone)]
enum TrafficWrite {
    Upsert(TrafficUsageState),
    Delete(String),
}

impl TrafficWrite {
    fn node_id(&self) -> &str {
        match self {
            Self::Upsert(state) => &state.usage.node_id,
            Self::Delete(node_id) => node_id,
        }
    }
}

/// 要求 writer 立即落盘并回报结果，让删除在返回前成为持久屏障。
struct TrafficFlushBarrier {
    completed: oneshot::Sender<Result<(), TrafficPersistenceError>>,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum TrafficPersistenceError {
    #[error("traffic writer is unavailable")]
    Unavailable,
    #[error("traffic persistence failed: {0}")]
    Write(#[from] anyhow::Error),
}

/// 独立于历史趋势 writer 的套餐账本。两者共享 SQLite/WAL，但不会让流量计数
/// 被历史写入节流，从而确保告警和限速使用每个 metrics 帧的最新总量。
#[derive(Clone)]
pub(super) struct TrafficTracker {
    db_path: Arc<PathBuf>,
    sqlite_busy_timeout_secs: u64,
    state: Arc<Mutex<HashMap<String, TrafficUsageState>>>,
    writer_tx: Arc<RwLock<Option<mpsc::Sender<TrafficFlushBarrier>>>>,
    writer_handle: Arc<Mutex<Option<JoinHandle<()>>>>,
    /// 有变更待落盘的节点。这里只记节点，由 writer 落盘时读取最新状态：内存受节点数约束，
    /// 也不存在排队中的旧内容覆盖新状态、或让已删除账本复活的问题。
    dirty: Arc<ParkingLotMutex<HashSet<String>>>,
    health: Arc<ParkingLotMutex<TrafficLedgerHealth>>,
    load_retry_handle: Arc<Mutex<Option<JoinHandle<()>>>>,
    recovered_after_startup: Arc<AtomicBool>,
}

impl TrafficTracker {
    pub(super) fn new(db_path: Arc<PathBuf>, sqlite_busy_timeout_secs: u64) -> Self {
        Self {
            db_path,
            sqlite_busy_timeout_secs,
            state: Arc::new(Mutex::new(HashMap::new())),
            writer_tx: Arc::new(RwLock::new(None)),
            writer_handle: Arc::new(Mutex::new(None)),
            dirty: Arc::new(ParkingLotMutex::new(HashSet::new())),
            health: Arc::new(ParkingLotMutex::new(TrafficLedgerHealth::default())),
            load_retry_handle: Arc::new(Mutex::new(None)),
            recovered_after_startup: Arc::new(AtomicBool::new(false)),
        }
    }

    pub(super) fn health(&self) -> TrafficLedgerHealth {
        *self.health.lock()
    }

    /// 加载账本；失败时后台重试，让锁库或被修复的数据不必重启服务就能恢复。
    pub(super) async fn start(&self, retry_interval: Duration) {
        let Err(error) = self.initialize().await else {
            return;
        };
        warn!(error = ?error, "traffic usage ledger unavailable; quota enforcement holds until it loads");
        let tracker = self.clone();
        let handle = tokio::spawn(async move {
            loop {
                tokio::time::sleep(retry_interval).await;
                match tracker.initialize().await {
                    Ok(()) => {
                        info!("traffic usage ledger loaded after retry");
                        tracker
                            .recovered_after_startup
                            .store(true, Ordering::Release);
                        return;
                    }
                    Err(error) => {
                        warn!(error = ?error, "traffic usage ledger still unavailable");
                    }
                }
            }
        });
        *self.load_retry_handle.lock().await = Some(handle);
    }

    /// 启动时的孤立账本清理在账本未加载时无法生效，恢复后调用方需要重新清理一次。
    pub(super) fn take_recovered_after_startup(&self) -> bool {
        self.recovered_after_startup.swap(false, Ordering::AcqRel)
    }

    /// 对账失败时恢复标记，让下一轮继续清理。
    pub(super) fn mark_recovered_after_startup(&self) {
        self.recovered_after_startup.store(true, Ordering::Release);
    }

    pub(super) async fn initialize(&self) -> Result<()> {
        let db_path = Arc::clone(&self.db_path);
        let timeout_secs = self.sqlite_busy_timeout_secs;
        let state =
            tokio::task::spawn_blocking(move || load_traffic_state(db_path.as_ref(), timeout_secs))
                .await
                .context("traffic usage database initialization task failed")??;
        *self.state.lock().await = state;

        let (tx, rx) = bounded_mpsc_channel(TRAFFIC_WRITER_CHANNEL_CAPACITY);
        *self.writer_tx.write().await = Some(tx);
        self.health.lock().available = true;
        let handle = tokio::spawn(run_traffic_writer(rx, self.clone()));
        *self.writer_handle.lock().await = Some(handle);
        Ok(())
    }

    pub(super) async fn record(
        &self,
        node_id: &str,
        sample: TrafficSample,
        accounting: TrafficAccounting,
        enabled: bool,
    ) -> Result<Option<TrafficUsage>, TrafficPersistenceError> {
        // 账本未加载时内存用量从 0 开始，据此做配额判断会把超额节点当成未超额。
        if self.writer_tx.read().await.is_none() {
            return Err(TrafficPersistenceError::Unavailable);
        }
        if !enabled {
            if self.state.lock().await.remove(node_id).is_some() {
                self.dirty.lock().insert(node_id.to_string());
            }
            return Ok(None);
        }

        let usage = {
            let mut state = self.state.lock().await;

            let now = Utc::now();
            let cycle_started_at = utc_month_start(now);
            let entry = state
                .entry(node_id.to_string())
                .or_insert_with(|| TrafficUsageState {
                    usage: TrafficUsage {
                        node_id: node_id.to_string(),
                        cycle_started_at,
                        used_bytes: 0,
                        accounting,
                    },
                    baseline: CounterBaseline::new(sample),
                });
            // 周期切换也经过 advance：临时回退期间不能把低读数当作新周期的基线。
            let (rx_delta, tx_delta) = entry.baseline.advance(sample);
            if entry.usage.cycle_started_at != cycle_started_at
                || entry.usage.accounting != accounting
            {
                entry.usage.cycle_started_at = cycle_started_at;
                entry.usage.accounting = accounting;
                entry.usage.used_bytes = 0;
            } else {
                entry.usage.used_bytes = entry
                    .usage
                    .used_bytes
                    .saturating_add(counted_bytes(accounting, rx_delta, tx_delta));
            }
            entry.usage.clone()
        };
        self.dirty.lock().insert(node_id.to_string());
        Ok(Some(usage))
    }

    pub(super) async fn usages(&self) -> Vec<TrafficUsage> {
        let mut usages = self
            .state
            .lock()
            .await
            .values()
            .map(|state| state.usage.clone())
            .collect::<Vec<_>>();
        usages.sort_by(|left, right| left.node_id.cmp(&right.node_id));
        usages
    }

    pub(super) async fn retain_nodes(
        &self,
        live_node_ids: &[String],
    ) -> Result<(), TrafficPersistenceError> {
        let live: std::collections::HashSet<&str> =
            live_node_ids.iter().map(String::as_str).collect();
        let removed = self
            .state
            .lock()
            .await
            .keys()
            .filter(|id| !live.contains(id.as_str()))
            .cloned()
            .collect();
        self.remove_nodes(removed).await
    }

    pub(super) async fn remove_nodes(
        &self,
        node_ids: Vec<String>,
    ) -> Result<(), TrafficPersistenceError> {
        if node_ids.is_empty() {
            return Ok(());
        }
        let tx = self
            .writer_tx
            .read()
            .await
            .as_ref()
            .cloned()
            .ok_or(TrafficPersistenceError::Unavailable)?;
        {
            let mut state = self.state.lock().await;
            for id in &node_ids {
                state.remove(id);
            }
        }
        self.dirty.lock().extend(node_ids);
        let (completed, receiver) = oneshot::channel();
        // writer 落盘前要取 state 锁，所以等待队列空位时绝不能持有它。
        tx.send(TrafficFlushBarrier { completed })
            .await
            .map_err(|_| TrafficPersistenceError::Unavailable)?;
        receiver
            .await
            .map_err(|_| TrafficPersistenceError::Unavailable)?
    }

    pub(super) async fn shutdown(&self) {
        if let Some(handle) = self.load_retry_handle.lock().await.take() {
            handle.abort();
            let _ = handle.await;
        }
        let sender = self.writer_tx.write().await.take();
        drop(sender);
        self.health.lock().available = false;
        if let Some(handle) = self.writer_handle.lock().await.take()
            && let Err(error) = handle.await
        {
            warn!(error = ?error, "traffic usage writer task join failed during shutdown");
        }
    }
}

fn counted_bytes(accounting: TrafficAccounting, rx_delta: u64, tx_delta: u64) -> u64 {
    match accounting {
        TrafficAccounting::Bidirectional => rx_delta.saturating_add(tx_delta),
        TrafficAccounting::Inbound => rx_delta,
        TrafficAccounting::Outbound => tx_delta,
    }
}

fn utc_month_start(now: DateTime<Utc>) -> DateTime<Utc> {
    match Utc
        .with_ymd_and_hms(now.year(), now.month(), 1, 0, 0, 0)
        .single()
    {
        Some(month_start) => month_start,
        None => now,
    }
}

fn load_traffic_state(
    db_path: &Path,
    sqlite_busy_timeout_secs: u64,
) -> Result<HashMap<String, TrafficUsageState>> {
    let connection = open_traffic_connection(db_path, sqlite_busy_timeout_secs)?;
    ensure_baseline_columns(&connection)?;
    let mut statement = connection.prepare(
        "SELECT node_id, cycle_started_at, accounting, used_bytes, last_rx_bytes, last_tx_bytes, last_uptime_secs, rx_high_water, tx_high_water FROM traffic_usage",
    )?;
    let rows = statement.query_map([], |row| {
        let node_id: String = row.get(0)?;
        let cycle_started_at: i64 = row.get(1)?;
        let accounting: String = row.get(2)?;
        let used_bytes: i64 = row.get(3)?;
        let last_rx_bytes: i64 = row.get(4)?;
        let last_tx_bytes: i64 = row.get(5)?;
        let last_uptime_secs: Option<i64> = row.get(6)?;
        let rx_high_water: Option<i64> = row.get(7)?;
        let tx_high_water: Option<i64> = row.get(8)?;
        Ok((
            node_id,
            cycle_started_at,
            accounting,
            used_bytes,
            last_rx_bytes,
            last_tx_bytes,
            last_uptime_secs,
            rx_high_water,
            tx_high_water,
        ))
    })?;
    let mut states = HashMap::new();
    for row in rows {
        let (
            node_id,
            cycle_started_at,
            accounting,
            used_bytes,
            last_rx_bytes,
            last_tx_bytes,
            last_uptime_secs,
            rx_high_water,
            tx_high_water,
        ) = row?;
        // 跳过坏行等于把该节点的用量清零，下一次写入还会覆盖原值，所以整体判定为不可用。
        let cycle_started_at = Utc
            .timestamp_opt(cycle_started_at, 0)
            .single()
            .with_context(|| format!("invalid traffic cycle start for node {node_id}"))?;
        let accounting = accounting_from_db(&accounting)
            .with_context(|| format!("invalid traffic accounting for node {node_id}"))?;
        states.insert(
            node_id.clone(),
            TrafficUsageState {
                usage: TrafficUsage {
                    node_id: node_id.clone(),
                    cycle_started_at,
                    used_bytes: stored_counter(used_bytes, &node_id)?,
                    accounting,
                },
                baseline: CounterBaseline::restored(
                    stored_counter(last_rx_bytes, &node_id)?,
                    stored_counter(last_tx_bytes, &node_id)?,
                    last_uptime_secs
                        .map(|uptime| stored_counter(uptime, &node_id))
                        .transpose()?,
                    rx_high_water
                        .map(|mark| stored_counter(mark, &node_id))
                        .transpose()?,
                    tx_high_water
                        .map(|mark| stored_counter(mark, &node_id))
                        .transpose()?,
                ),
            },
        );
    }
    Ok(states)
}

/// 失败重试的退避状态。待写内容按节点合并，内存受节点数约束，因此可以一直重试。
#[derive(Default)]
struct RetryBackoff {
    delay: Option<Duration>,
    next_attempt_at: Option<Instant>,
}

impl RetryBackoff {
    fn ready(&self) -> bool {
        self.next_attempt_at.is_none_or(|at| Instant::now() >= at)
    }

    fn record_failure(&mut self) {
        let delay = self.delay.map_or(TRAFFIC_RETRY_INITIAL_DELAY, |delay| {
            delay.saturating_mul(2).min(TRAFFIC_RETRY_MAX_DELAY)
        });
        self.delay = Some(delay);
        self.next_attempt_at = Some(Instant::now() + delay);
    }

    fn reset(&mut self) {
        *self = Self::default();
    }
}

async fn run_traffic_writer(mut rx: mpsc::Receiver<TrafficFlushBarrier>, tracker: TrafficTracker) {
    let mut pending = HashMap::new();
    let mut backoff = RetryBackoff::default();
    let mut ticker = interval(Duration::from_millis(TRAFFIC_WRITER_FLUSH_MILLIS));
    ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
    ticker.tick().await;
    loop {
        tokio::select! {
            received = rx.recv() => match received {
                Some(TrafficFlushBarrier { completed }) => {
                    collect_dirty_writes(&mut pending, &tracker).await;
                    let result = flush_traffic_writes(&mut pending, &tracker, &mut backoff).await;
                    let _ = completed.send(result.map_err(TrafficPersistenceError::Write));
                }
                None => break,
            },
            _ = ticker.tick() => {
                collect_dirty_writes(&mut pending, &tracker).await;
                if !pending.is_empty()
                    && backoff.ready()
                    && let Err(error) = flush_traffic_writes(&mut pending, &tracker, &mut backoff).await
                {
                    warn!(error = ?error, pending = pending.len(), "failed to persist traffic usage state; will retry");
                }
            },
        }
        tracker.health.lock().pending_writes = pending.len() as u64;
    }
    collect_dirty_writes(&mut pending, &tracker).await;
    if !pending.is_empty()
        && let Err(error) = flush_traffic_writes(&mut pending, &tracker, &mut backoff).await
    {
        error!(
            error = ?error,
            unpersisted_nodes = pending.len(),
            "traffic usage state not persisted at shutdown; usage since the last commit is recovered from counters on reconnect",
        );
    }
    tracker.health.lock().pending_writes = pending.len() as u64;
}

/// 把变更节点解析为当前状态：已不存在的节点写删除，覆盖重试中保留的旧内容。
async fn collect_dirty_writes(
    pending: &mut HashMap<String, TrafficWrite>,
    tracker: &TrafficTracker,
) {
    let dirty = std::mem::take(&mut *tracker.dirty.lock());
    if dirty.is_empty() {
        return;
    }
    let state = tracker.state.lock().await;
    for node_id in dirty {
        let write = match state.get(&node_id) {
            Some(usage) => TrafficWrite::Upsert(usage.clone()),
            None => TrafficWrite::Delete(node_id),
        };
        record_pending(pending, write);
    }
}

fn record_pending(pending: &mut HashMap<String, TrafficWrite>, write: TrafficWrite) {
    pending.insert(write.node_id().to_string(), write);
}

async fn flush_traffic_writes(
    pending: &mut HashMap<String, TrafficWrite>,
    tracker: &TrafficTracker,
    backoff: &mut RetryBackoff,
) -> Result<()> {
    let writes = Arc::new(std::mem::take(pending));
    let db_path = Arc::clone(&tracker.db_path);
    let timeout_secs = tracker.sqlite_busy_timeout_secs;
    let task_writes = Arc::clone(&writes);
    let result = tokio::task::spawn_blocking(move || {
        write_traffic_states(db_path.as_ref(), timeout_secs, &task_writes)
    })
    .await
    .context("traffic writer task failed")
    .and_then(|result| result);
    if result.is_err() {
        // 写入的是绝对值，重试幂等；flush 期间 pending 不会变化，无需与新内容合并。
        *pending = Arc::unwrap_or_clone(writes);
    }
    match &result {
        Ok(()) => {
            let recovered = tracker.health.lock().write_degraded;
            tracker.health.lock().record_success();
            backoff.reset();
            if recovered {
                info!("traffic usage persistence recovered");
            }
        }
        Err(_) => {
            tracker.health.lock().record_failure();
            backoff.record_failure();
        }
    }
    result
}

fn write_traffic_states(
    db_path: &Path,
    sqlite_busy_timeout_secs: u64,
    writes: &HashMap<String, TrafficWrite>,
) -> Result<()> {
    let mut connection = open_traffic_connection(db_path, sqlite_busy_timeout_secs)?;
    let tx = connection.transaction()?;
    for write in writes.values() {
        match write {
            TrafficWrite::Upsert(state) => {
                tx.execute(
                    "INSERT INTO traffic_usage (node_id, cycle_started_at, accounting, used_bytes, last_rx_bytes, last_tx_bytes, last_uptime_secs, rx_high_water, tx_high_water) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9) ON CONFLICT(node_id) DO UPDATE SET cycle_started_at = excluded.cycle_started_at, accounting = excluded.accounting, used_bytes = excluded.used_bytes, last_rx_bytes = excluded.last_rx_bytes, last_tx_bytes = excluded.last_tx_bytes, last_uptime_secs = excluded.last_uptime_secs, rx_high_water = excluded.rx_high_water, tx_high_water = excluded.tx_high_water",
                    params![
                        state.usage.node_id,
                        state.usage.cycle_started_at.timestamp(),
                        accounting_to_db(state.usage.accounting),
                        u64_to_sqlite_integer(state.usage.used_bytes),
                        u64_to_sqlite_integer(state.baseline.rx_bytes),
                        u64_to_sqlite_integer(state.baseline.tx_bytes),
                        state.baseline.uptime_secs.map(u64_to_sqlite_integer),
                        state.baseline.rx_high_water.map(u64_to_sqlite_integer),
                        state.baseline.tx_high_water.map(u64_to_sqlite_integer),
                    ],
                )?;
            }
            TrafficWrite::Delete(node_id) => {
                tx.execute(
                    "DELETE FROM traffic_usage WHERE node_id = ?1",
                    params![node_id],
                )?;
            }
        }
    }
    tx.commit()?;
    Ok(())
}

fn open_traffic_connection(db_path: &Path, sqlite_busy_timeout_secs: u64) -> Result<Connection> {
    let connection = Connection::open(db_path).with_context(|| {
        format!(
            "failed to open traffic usage database {}",
            db_path.display()
        )
    })?;
    connection
        .busy_timeout(std::time::Duration::from_secs(sqlite_busy_timeout_secs))
        .context("failed to configure traffic usage SQLite busy timeout")?;
    connection.execute_batch(
        "PRAGMA journal_mode = WAL;\
         CREATE TABLE IF NOT EXISTS traffic_usage (\
             node_id TEXT PRIMARY KEY NOT NULL,\
             cycle_started_at INTEGER NOT NULL,\
             accounting TEXT NOT NULL,\
             used_bytes INTEGER NOT NULL,\
             last_rx_bytes INTEGER NOT NULL,\
             last_tx_bytes INTEGER NOT NULL,\
             last_uptime_secs INTEGER,\
             rx_high_water INTEGER,\
             tx_high_water INTEGER\
         );",
    )?;
    Ok(connection)
}

fn ensure_baseline_columns(connection: &Connection) -> Result<()> {
    const COLUMNS: [(&str, &str); 3] = [
        (
            "last_uptime_secs",
            "ALTER TABLE traffic_usage ADD COLUMN last_uptime_secs INTEGER;",
        ),
        (
            "rx_high_water",
            "ALTER TABLE traffic_usage ADD COLUMN rx_high_water INTEGER;",
        ),
        (
            "tx_high_water",
            "ALTER TABLE traffic_usage ADD COLUMN tx_high_water INTEGER;",
        ),
    ];
    for (name, add_column) in COLUMNS {
        let exists = connection
            .prepare("SELECT 1 FROM pragma_table_info('traffic_usage') WHERE name = ?1")?
            .exists([name])?;
        if !exists {
            connection
                .execute_batch(add_column)
                .with_context(|| format!("failed to add {name} to traffic usage table"))?;
        }
    }
    Ok(())
}

fn accounting_to_db(accounting: TrafficAccounting) -> &'static str {
    match accounting {
        TrafficAccounting::Bidirectional => "bidirectional",
        TrafficAccounting::Inbound => "inbound",
        TrafficAccounting::Outbound => "outbound",
    }
}

fn accounting_from_db(value: &str) -> Option<TrafficAccounting> {
    match value {
        "bidirectional" => Some(TrafficAccounting::Bidirectional),
        "inbound" => Some(TrafficAccounting::Inbound),
        "outbound" => Some(TrafficAccounting::Outbound),
        _ => None,
    }
}

fn u64_to_sqlite_integer(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

fn stored_counter(value: i64, node_id: &str) -> Result<u64> {
    u64::try_from(value).with_context(|| format!("negative traffic counter for node {node_id}"))
}

#[cfg(test)]
mod tests;
