use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::{Context, Result};
use chrono::{DateTime, TimeZone, Utc};
use rusqlite::{Connection, params};
use tokio::sync::Mutex;
use tracing::warn;
use ximonitor_proto::{
    DEFAULT_HISTORY_RETENTION_HOURS, DEFAULT_HISTORY_WRITE_INTERVAL_SECS, HistoryPoint, NodeStatus,
    percentage,
};

#[derive(Clone)]
pub struct HistoryStore {
    db_path: Arc<PathBuf>,
    available: Arc<AtomicBool>,
    last_written_at: Arc<Mutex<HashMap<String, DateTime<Utc>>>>,
    last_pruned_at: Arc<Mutex<Option<DateTime<Utc>>>>,
}

impl HistoryStore {
    pub fn new(db_path: PathBuf) -> Self {
        Self {
            db_path: Arc::new(db_path),
            available: Arc::new(AtomicBool::new(false)),
            last_written_at: Arc::new(Mutex::new(HashMap::new())),
            last_pruned_at: Arc::new(Mutex::new(None)),
        }
    }

    pub async fn initialize(&self) {
        let db_path = Arc::clone(&self.db_path);
        let result = tokio::task::spawn_blocking(move || initialize_database(db_path.as_ref()))
            .await
            .context("history database task failed");

        match result {
            Ok(Ok(())) => {
                self.available.store(true, Ordering::Relaxed);
            }
            Ok(Err(error)) => {
                warn!(error = ?error, "history database unavailable; real-time views will continue");
            }
            Err(error) => {
                warn!(error = ?error, "history database initialization join failed");
            }
        }
    }

    pub fn is_available(&self) -> bool {
        self.available.load(Ordering::Relaxed)
    }

    pub async fn record_status(&self, status: &NodeStatus) {
        if !self.is_available() {
            return;
        }

        let Some(point) = build_history_point(status) else {
            return;
        };

        {
            let guard = self.last_written_at.lock().await;
            if let Some(previous) = guard.get(&point.node_id) {
                let Ok(elapsed) = point
                    .recorded_at
                    .signed_duration_since(previous.to_owned())
                    .to_std()
                else {
                    return;
                };
                if elapsed < Duration::from_secs(DEFAULT_HISTORY_WRITE_INTERVAL_SECS) {
                    return;
                }
            }
        }

        let prune_before = self.maybe_schedule_prune().await;
        let db_path = Arc::clone(&self.db_path);
        let point_for_task = point.clone();
        let result = tokio::task::spawn_blocking(move || {
            write_history_point(db_path.as_ref(), &point_for_task, prune_before)
        })
        .await;

        match result {
            Ok(Ok(())) => {
                let mut guard = self.last_written_at.lock().await;
                guard.insert(point.node_id, point.recorded_at);
            }
            Ok(Err(error)) => {
                warn!(error = ?error, "failed to persist history point");
            }
            Err(error) => {
                warn!(error = ?error, "history write task join failed");
            }
        }
    }

    pub async fn query_recent_history(&self, node_id: &str) -> Result<Vec<HistoryPoint>> {
        if !self.is_available() {
            return Ok(Vec::new());
        }

        let db_path = Arc::clone(&self.db_path);
        let node_id = node_id.to_string();
        let since = Utc::now() - chrono::Duration::hours(DEFAULT_HISTORY_RETENTION_HOURS as i64);

        tokio::task::spawn_blocking(move || query_history(db_path.as_ref(), &node_id, since))
            .await
            .context("history query task failed")?
    }

    async fn maybe_schedule_prune(&self) -> Option<DateTime<Utc>> {
        let mut guard = self.last_pruned_at.lock().await;
        let now = Utc::now();
        let should_prune = guard
            .as_ref()
            .map(|last_pruned| {
                now.signed_duration_since(last_pruned.to_owned())
                    .to_std()
                    .map(|elapsed| elapsed >= Duration::from_secs(300))
                    .unwrap_or(false)
            })
            .unwrap_or(true);

        if should_prune {
            *guard = Some(now);
            Some(now - chrono::Duration::hours(DEFAULT_HISTORY_RETENTION_HOURS as i64))
        } else {
            None
        }
    }
}

fn initialize_database(db_path: &PathBuf) -> Result<()> {
    if let Some(parent) = db_path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).with_context(|| {
                format!("failed to create history directory {}", parent.display())
            })?;
        }
    }

    let connection = Connection::open(db_path)
        .with_context(|| format!("failed to open history database {}", db_path.display()))?;
    connection.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS history_points (
            node_id TEXT NOT NULL,
            recorded_at INTEGER NOT NULL,
            cpu_usage_percent REAL NOT NULL,
            memory_used_percent REAL NOT NULL,
            rx_bytes_per_sec REAL,
            tx_bytes_per_sec REAL,
            latency_ms INTEGER,
            disk_used_percent REAL
        );
        CREATE INDEX IF NOT EXISTS idx_history_points_node_time
            ON history_points (node_id, recorded_at);
        "#,
    )?;

    Ok(())
}

fn write_history_point(
    db_path: &PathBuf,
    point: &HistoryPoint,
    prune_before: Option<DateTime<Utc>>,
) -> Result<()> {
    let connection = Connection::open(db_path)
        .with_context(|| format!("failed to open history database {}", db_path.display()))?;
    connection.execute(
        r#"
        INSERT INTO history_points (
            node_id,
            recorded_at,
            cpu_usage_percent,
            memory_used_percent,
            rx_bytes_per_sec,
            tx_bytes_per_sec,
            latency_ms,
            disk_used_percent
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
        "#,
        params![
            &point.node_id,
            point.recorded_at.timestamp(),
            point.cpu_usage_percent,
            point.memory_used_percent,
            point.rx_bytes_per_sec,
            point.tx_bytes_per_sec,
            point.latency_ms,
            point.disk_used_percent,
        ],
    )?;

    if let Some(cutoff) = prune_before {
        connection.execute(
            "DELETE FROM history_points WHERE recorded_at < ?1",
            params![cutoff.timestamp()],
        )?;
    }

    Ok(())
}

fn query_history(
    db_path: &PathBuf,
    node_id: &str,
    since: DateTime<Utc>,
) -> Result<Vec<HistoryPoint>> {
    let connection = Connection::open(db_path)
        .with_context(|| format!("failed to open history database {}", db_path.display()))?;
    let mut statement = connection.prepare(
        r#"
        SELECT
            node_id,
            recorded_at,
            cpu_usage_percent,
            memory_used_percent,
            rx_bytes_per_sec,
            tx_bytes_per_sec,
            latency_ms,
            disk_used_percent
        FROM history_points
        WHERE node_id = ?1 AND recorded_at >= ?2
        ORDER BY recorded_at ASC
        "#,
    )?;
    let rows = statement.query_map(params![node_id, since.timestamp()], |row| {
        let recorded_at = row.get::<_, i64>(1)?;
        Ok(HistoryPoint {
            node_id: row.get(0)?,
            recorded_at: Utc
                .timestamp_opt(recorded_at, 0)
                .single()
                .unwrap_or_else(Utc::now),
            cpu_usage_percent: row.get(2)?,
            memory_used_percent: row.get(3)?,
            rx_bytes_per_sec: row.get(4)?,
            tx_bytes_per_sec: row.get(5)?,
            latency_ms: row.get(6)?,
            disk_used_percent: row.get(7)?,
        })
    })?;

    let mut points = Vec::new();
    for row in rows {
        points.push(row?);
    }
    Ok(points)
}

fn build_history_point(status: &NodeStatus) -> Option<HistoryPoint> {
    let snapshot = status.snapshot.as_ref()?;
    let total_disk_bytes: u64 = snapshot.disks.iter().map(|disk| disk.total_bytes).sum();
    let used_disk_bytes: u64 = snapshot.disks.iter().map(|disk| disk.used_bytes).sum();
    let disk_used_percent =
        (total_disk_bytes > 0).then(|| percentage(used_disk_bytes, total_disk_bytes));

    Some(HistoryPoint {
        node_id: status.identity.node_id.clone(),
        recorded_at: snapshot.collected_at,
        cpu_usage_percent: snapshot.cpu_usage_percent,
        memory_used_percent: snapshot.memory.used_percent(),
        rx_bytes_per_sec: snapshot.network.rx_bytes_per_sec,
        tx_bytes_per_sec: snapshot.network.tx_bytes_per_sec,
        latency_ms: status.latency_ms,
        disk_used_percent,
    })
}
