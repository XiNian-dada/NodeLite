//! 按自然月累计节点套餐流量，并把计数器状态异步写入历史 SQLite 数据库。
//!
//! Agent 的网卡累计字节会在重启时归零，因此这里保存每个节点上一次看到的
//! 收发计数；只累加单调递增的差值。计费口径或自然月变化时重建基线，避免把
//! 不同套餐的字节数混合在同一个使用量中。

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use chrono::{DateTime, Datelike, TimeZone, Utc};
use rusqlite::{Connection, params};
use tokio::sync::{Mutex, RwLock, mpsc};
use tokio::task::JoinHandle;
use tokio::time::{MissedTickBehavior, interval};
use tracing::warn;

use crate::queue::{bounded_mpsc_channel, try_enqueue};
use crate::registry::TrafficAccounting;

const TRAFFIC_WRITER_CHANNEL_CAPACITY: usize = 1024;
const TRAFFIC_WRITER_FLUSH_MILLIS: u64 = 200;

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
    last_rx_bytes: u64,
    last_tx_bytes: u64,
}

#[derive(Debug, Clone)]
enum TrafficWrite {
    Upsert(TrafficUsageState),
    Delete(String),
}

/// 独立于历史趋势 writer 的套餐账本。两者共享 SQLite/WAL，但不会让流量计数
/// 被历史写入节流，从而确保告警和限速使用每个 metrics 帧的最新总量。
#[derive(Clone)]
pub(super) struct TrafficTracker {
    db_path: Arc<PathBuf>,
    sqlite_busy_timeout_secs: u64,
    state: Arc<Mutex<HashMap<String, TrafficUsageState>>>,
    writer_tx: Arc<RwLock<Option<mpsc::Sender<TrafficWrite>>>>,
    writer_handle: Arc<Mutex<Option<JoinHandle<()>>>>,
}

impl TrafficTracker {
    pub(super) fn new(db_path: Arc<PathBuf>, sqlite_busy_timeout_secs: u64) -> Self {
        Self {
            db_path,
            sqlite_busy_timeout_secs,
            state: Arc::new(Mutex::new(HashMap::new())),
            writer_tx: Arc::new(RwLock::new(None)),
            writer_handle: Arc::new(Mutex::new(None)),
        }
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
        let db_path = Arc::clone(&self.db_path);
        let handle = tokio::spawn(run_traffic_writer(
            rx,
            db_path,
            self.sqlite_busy_timeout_secs,
        ));
        *self.writer_handle.lock().await = Some(handle);
        Ok(())
    }

    pub(super) async fn record(
        &self,
        node_id: &str,
        total_rx_bytes: u64,
        total_tx_bytes: u64,
        accounting: TrafficAccounting,
        enabled: bool,
    ) -> Option<TrafficUsage> {
        let removed = {
            let mut state = self.state.lock().await;
            if !enabled {
                state
                    .remove(node_id)
                    .map(|_| TrafficWrite::Delete(node_id.to_string()))
            } else {
                None
            }
        };
        if !enabled {
            if let Some(write) = removed {
                self.try_enqueue(write).await;
            }
            return None;
        }

        let write = {
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
                    last_rx_bytes: total_rx_bytes,
                    last_tx_bytes: total_tx_bytes,
                });
            if entry.usage.cycle_started_at != cycle_started_at
                || entry.usage.accounting != accounting
            {
                *entry = TrafficUsageState {
                    usage: TrafficUsage {
                        node_id: node_id.to_string(),
                        cycle_started_at,
                        used_bytes: 0,
                        accounting,
                    },
                    last_rx_bytes: total_rx_bytes,
                    last_tx_bytes: total_tx_bytes,
                };
            } else {
                let rx_delta = total_rx_bytes.saturating_sub(entry.last_rx_bytes);
                let tx_delta = total_tx_bytes.saturating_sub(entry.last_tx_bytes);
                entry.usage.used_bytes = entry
                    .usage
                    .used_bytes
                    .saturating_add(counted_bytes(accounting, rx_delta, tx_delta));
                entry.last_rx_bytes = total_rx_bytes;
                entry.last_tx_bytes = total_tx_bytes;
            }
            TrafficWrite::Upsert(entry.clone())
        };
        let usage = match &write {
            TrafficWrite::Upsert(state) => state.usage.clone(),
            TrafficWrite::Delete(_) => return None,
        };
        self.try_enqueue(write).await;
        Some(usage)
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

    pub(super) async fn shutdown(&self) {
        let sender = self.writer_tx.write().await.take();
        drop(sender);
        if let Some(handle) = self.writer_handle.lock().await.take()
            && let Err(error) = handle.await
        {
            warn!(error = ?error, "traffic usage writer task join failed during shutdown");
        }
    }

    async fn try_enqueue(&self, write: TrafficWrite) {
        let tx = self.writer_tx.read().await.as_ref().cloned();
        if let Some(tx) = tx {
            let _ = try_enqueue(&tx, write);
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
    db_path: &PathBuf,
    sqlite_busy_timeout_secs: u64,
) -> Result<HashMap<String, TrafficUsageState>> {
    let connection = open_traffic_connection(db_path, sqlite_busy_timeout_secs)?;
    let mut statement = connection.prepare(
        "SELECT node_id, cycle_started_at, accounting, used_bytes, last_rx_bytes, last_tx_bytes FROM traffic_usage",
    )?;
    let rows = statement.query_map([], |row| {
        let node_id: String = row.get(0)?;
        let cycle_started_at: i64 = row.get(1)?;
        let accounting: String = row.get(2)?;
        let used_bytes: i64 = row.get(3)?;
        let last_rx_bytes: i64 = row.get(4)?;
        let last_tx_bytes: i64 = row.get(5)?;
        Ok((
            node_id,
            cycle_started_at,
            accounting,
            used_bytes,
            last_rx_bytes,
            last_tx_bytes,
        ))
    })?;
    let mut states = HashMap::new();
    for row in rows {
        let (node_id, cycle_started_at, accounting, used_bytes, last_rx_bytes, last_tx_bytes) =
            row?;
        let Some(cycle_started_at) = Utc.timestamp_opt(cycle_started_at, 0).single() else {
            continue;
        };
        let Some(accounting) = accounting_from_db(&accounting) else {
            continue;
        };
        states.insert(
            node_id.clone(),
            TrafficUsageState {
                usage: TrafficUsage {
                    node_id,
                    cycle_started_at,
                    used_bytes: sqlite_integer_to_u64(used_bytes),
                    accounting,
                },
                last_rx_bytes: sqlite_integer_to_u64(last_rx_bytes),
                last_tx_bytes: sqlite_integer_to_u64(last_tx_bytes),
            },
        );
    }
    Ok(states)
}

async fn run_traffic_writer(
    mut rx: mpsc::Receiver<TrafficWrite>,
    db_path: Arc<PathBuf>,
    sqlite_busy_timeout_secs: u64,
) {
    let mut pending = HashMap::new();
    let mut ticker = interval(std::time::Duration::from_millis(
        TRAFFIC_WRITER_FLUSH_MILLIS,
    ));
    ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
    ticker.tick().await;
    loop {
        tokio::select! {
            received = rx.recv() => match received {
                Some(write) => record_pending(&mut pending, write),
                None => break,
            },
            _ = ticker.tick(), if !pending.is_empty() => flush_traffic_writes(&mut pending, db_path.as_ref(), sqlite_busy_timeout_secs).await,
        }
    }
    while let Ok(write) = rx.try_recv() {
        record_pending(&mut pending, write);
    }
    if !pending.is_empty() {
        flush_traffic_writes(&mut pending, db_path.as_ref(), sqlite_busy_timeout_secs).await;
    }
}

fn record_pending(pending: &mut HashMap<String, TrafficWrite>, write: TrafficWrite) {
    let node_id = match &write {
        TrafficWrite::Upsert(state) => state.usage.node_id.clone(),
        TrafficWrite::Delete(node_id) => node_id.clone(),
    };
    pending.insert(node_id, write);
}

async fn flush_traffic_writes(
    pending: &mut HashMap<String, TrafficWrite>,
    db_path: &PathBuf,
    sqlite_busy_timeout_secs: u64,
) {
    let writes = std::mem::take(pending);
    let db_path = db_path.clone();
    let result = tokio::task::spawn_blocking(move || {
        write_traffic_states(&db_path, sqlite_busy_timeout_secs, writes)
    })
    .await;
    if let Ok(Err(error)) = result {
        warn!(error = ?error, "failed to persist traffic usage state");
    }
}

fn write_traffic_states(
    db_path: &PathBuf,
    sqlite_busy_timeout_secs: u64,
    writes: HashMap<String, TrafficWrite>,
) -> Result<()> {
    let mut connection = open_traffic_connection(db_path, sqlite_busy_timeout_secs)?;
    let tx = connection.transaction()?;
    for write in writes.into_values() {
        match write {
            TrafficWrite::Upsert(state) => {
                tx.execute(
                    "INSERT INTO traffic_usage (node_id, cycle_started_at, accounting, used_bytes, last_rx_bytes, last_tx_bytes) VALUES (?1, ?2, ?3, ?4, ?5, ?6) ON CONFLICT(node_id) DO UPDATE SET cycle_started_at = excluded.cycle_started_at, accounting = excluded.accounting, used_bytes = excluded.used_bytes, last_rx_bytes = excluded.last_rx_bytes, last_tx_bytes = excluded.last_tx_bytes",
                    params![
                        state.usage.node_id,
                        state.usage.cycle_started_at.timestamp(),
                        accounting_to_db(state.usage.accounting),
                        u64_to_sqlite_integer(state.usage.used_bytes),
                        u64_to_sqlite_integer(state.last_rx_bytes),
                        u64_to_sqlite_integer(state.last_tx_bytes),
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

fn open_traffic_connection(db_path: &PathBuf, sqlite_busy_timeout_secs: u64) -> Result<Connection> {
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
             last_tx_bytes INTEGER NOT NULL\
         );",
    )?;
    Ok(connection)
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
    match i64::try_from(value) {
        Ok(value) => value,
        Err(_) => i64::MAX,
    }
}

fn sqlite_integer_to_u64(value: i64) -> u64 {
    match u64::try_from(value) {
        Ok(value) => value,
        Err(_) => 0,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::{SystemTime, UNIX_EPOCH};

    use chrono::{TimeZone, Utc};

    use super::{TrafficTracker, counted_bytes, utc_month_start};
    use crate::registry::TrafficAccounting;

    #[test]
    fn traffic_accounting_uses_the_configured_direction() {
        assert_eq!(counted_bytes(TrafficAccounting::Bidirectional, 10, 15), 25);
        assert_eq!(counted_bytes(TrafficAccounting::Inbound, 10, 15), 10);
        assert_eq!(counted_bytes(TrafficAccounting::Outbound, 10, 15), 15);
    }

    #[test]
    fn month_start_is_utc_not_the_server_local_timezone() {
        let now = Utc.with_ymd_and_hms(2026, 7, 30, 16, 0, 0).unwrap();
        assert_eq!(
            utc_month_start(now),
            Utc.with_ymd_and_hms(2026, 7, 1, 0, 0, 0).unwrap()
        );
    }

    #[tokio::test]
    async fn record_uses_counter_deltas_and_resets_when_accounting_changes() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be available")
            .as_nanos();
        let db_path = std::env::temp_dir().join(format!("nodelite-traffic-test-{unique}.sqlite3"));
        let tracker = TrafficTracker::new(Arc::new(db_path.clone()), 1);
        tracker
            .initialize()
            .await
            .expect("tracker should initialize");

        let baseline = tracker
            .record("node-a", 100, 200, TrafficAccounting::Bidirectional, true)
            .await
            .expect("enabled quota should return usage");
        assert_eq!(baseline.used_bytes, 0);
        let usage = tracker
            .record("node-a", 130, 260, TrafficAccounting::Bidirectional, true)
            .await
            .expect("usage should be recorded");
        assert_eq!(usage.used_bytes, 90);

        let reset = tracker
            .record("node-a", 140, 290, TrafficAccounting::Outbound, true)
            .await
            .expect("changed accounting should return usage");
        assert_eq!(reset.used_bytes, 0);
        let outbound = tracker
            .record("node-a", 160, 330, TrafficAccounting::Outbound, true)
            .await
            .expect("outbound usage should be recorded");
        assert_eq!(outbound.used_bytes, 40);

        tracker.shutdown().await;
        let _ = std::fs::remove_file(db_path);
    }
}
