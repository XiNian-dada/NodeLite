use std::collections::HashMap;
use std::ffi::OsString;
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

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

const SQLITE_BUSY_TIMEOUT_SECS: u64 = 5;

#[derive(Clone)]
pub struct HistoryStore {
    db_path: Arc<PathBuf>,
    available: Arc<AtomicBool>,
    last_written_at: Arc<Mutex<HashMap<String, DateTime<Utc>>>>,
    last_pruned_at: Arc<Mutex<Option<DateTime<Utc>>>>,
    write_gate: Arc<Mutex<()>>,
}

impl HistoryStore {
    pub fn new(db_path: PathBuf) -> Self {
        Self {
            db_path: Arc::new(db_path),
            available: Arc::new(AtomicBool::new(false)),
            last_written_at: Arc::new(Mutex::new(HashMap::new())),
            last_pruned_at: Arc::new(Mutex::new(None)),
            write_gate: Arc::new(Mutex::new(())),
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

        let _write_guard = self.write_gate.lock().await;
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

    pub async fn query_history(
        &self,
        node_id: &str,
        window_hours: u64,
        max_points: usize,
    ) -> Result<Vec<HistoryPoint>> {
        if !self.is_available() {
            return Ok(Vec::new());
        }

        let db_path = Arc::clone(&self.db_path);
        let node_id = node_id.to_string();
        let clamped_window_hours = window_hours.clamp(1, DEFAULT_HISTORY_RETENTION_HOURS);
        let clamped_max_points = max_points.max(60);
        let since = Utc::now() - chrono::Duration::hours(clamped_window_hours as i64);

        tokio::task::spawn_blocking(move || {
            query_history(db_path.as_ref(), &node_id, since, clamped_max_points)
        })
        .await
        .context("history query task failed")?
    }

    pub async fn query_history_range(
        &self,
        node_id: &str,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
        max_points: usize,
    ) -> Result<Vec<HistoryPoint>> {
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

        let db_path = Arc::clone(&self.db_path);
        let node_id = node_id.to_string();
        let clamped_max_points = max_points.max(60);

        tokio::task::spawn_blocking(move || {
            query_history_between(
                db_path.as_ref(),
                &node_id,
                clamped_start,
                clamped_end,
                clamped_max_points,
            )
        })
        .await
        .context("history range query task failed")?
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
    if let Some(parent) = db_path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create history directory {}", parent.display()))?;
    }

    let connection = open_database_connection(db_path, true)?;
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
    harden_database_artifacts(db_path)?;

    Ok(())
}

fn write_history_point(
    db_path: &PathBuf,
    point: &HistoryPoint,
    prune_before: Option<DateTime<Utc>>,
) -> Result<()> {
    let connection = open_database_connection(db_path, true)?;
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
    harden_database_artifacts(db_path)?;

    Ok(())
}

fn query_history(
    db_path: &PathBuf,
    node_id: &str,
    since: DateTime<Utc>,
    max_points: usize,
) -> Result<Vec<HistoryPoint>> {
    query_history_between(db_path, node_id, since, Utc::now(), max_points)
}

fn query_history_between(
    db_path: &PathBuf,
    node_id: &str,
    since: DateTime<Utc>,
    until: DateTime<Utc>,
    max_points: usize,
) -> Result<Vec<HistoryPoint>> {
    let connection = open_database_connection(db_path, false)?;
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
        WHERE node_id = ?1 AND recorded_at >= ?2 AND recorded_at <= ?3
        ORDER BY recorded_at ASC
        "#,
    )?;
    let rows = statement.query_map(
        params![node_id, since.timestamp(), until.timestamp()],
        |row| {
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
        },
    )?;

    let mut points = Vec::new();
    for row in rows {
        points.push(row?);
    }
    Ok(condense_history_points(points, max_points))
}

fn open_database_connection(db_path: &PathBuf, enable_wal: bool) -> Result<Connection> {
    let connection = Connection::open(db_path)
        .with_context(|| format!("failed to open history database {}", db_path.display()))?;
    connection
        .busy_timeout(Duration::from_secs(SQLITE_BUSY_TIMEOUT_SECS))
        .context("failed to configure sqlite busy timeout")?;
    if enable_wal {
        connection
            .pragma_update(None, "journal_mode", "WAL")
            .context("failed to enable sqlite WAL mode")?;
    }
    harden_database_artifacts(db_path)?;
    Ok(connection)
}

fn harden_database_artifacts(db_path: &PathBuf) -> Result<()> {
    harden_path_permissions(db_path)?;
    for suffix in ["-wal", "-shm"] {
        let mut artifact = OsString::from(db_path.as_os_str());
        artifact.push(suffix);
        let artifact = PathBuf::from(artifact);
        if artifact.exists() {
            harden_path_permissions(&artifact)?;
        }
    }
    Ok(())
}

fn harden_path_permissions(path: &PathBuf) -> Result<()> {
    #[cfg(unix)]
    {
        if path.exists() {
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
                .with_context(|| format!("failed to chmod {}", path.display()))?;
        }
    }

    #[cfg(not(unix))]
    {
        let _ = path;
    }

    Ok(())
}

fn build_history_point(status: &NodeStatus) -> Option<HistoryPoint> {
    let snapshot = status.snapshot.as_ref()?;
    let total_disk_bytes = snapshot
        .disks
        .iter()
        .fold(0_u64, |total, disk| total.saturating_add(disk.total_bytes));
    let used_disk_bytes = snapshot
        .disks
        .iter()
        .fold(0_u64, |total, disk| total.saturating_add(disk.used_bytes));
    let disk_used_percent =
        (total_disk_bytes > 0).then(|| percentage(used_disk_bytes, total_disk_bytes));
    let recorded_at = status.last_seen.unwrap_or_else(Utc::now);

    Some(HistoryPoint {
        node_id: status.identity.node_id.clone(),
        recorded_at,
        cpu_usage_percent: snapshot.cpu_usage_percent,
        memory_used_percent: snapshot.memory.used_percent(),
        rx_bytes_per_sec: snapshot.network.rx_bytes_per_sec,
        tx_bytes_per_sec: snapshot.network.tx_bytes_per_sec,
        latency_ms: status.latency_ms,
        disk_used_percent,
    })
}

fn condense_history_points(points: Vec<HistoryPoint>, max_points: usize) -> Vec<HistoryPoint> {
    let target_points = max_points.max(1);
    if points.len() <= target_points {
        return points;
    }

    let bucket_size = points.len().div_ceil(target_points);
    let mut condensed = Vec::with_capacity(points.len().div_ceil(bucket_size));

    for chunk in points.chunks(bucket_size) {
        condensed.push(average_history_chunk(chunk));
    }

    condensed
}

fn average_history_chunk(chunk: &[HistoryPoint]) -> HistoryPoint {
    let first = &chunk[0];
    let recorded_at = chunk
        .last()
        .map(|point| point.recorded_at)
        .unwrap_or(first.recorded_at);

    HistoryPoint {
        node_id: first.node_id.clone(),
        recorded_at,
        cpu_usage_percent: average_f64(chunk.iter().map(|point| point.cpu_usage_percent)),
        memory_used_percent: average_f64(chunk.iter().map(|point| point.memory_used_percent)),
        rx_bytes_per_sec: average_optional_f64(chunk.iter().map(|point| point.rx_bytes_per_sec)),
        tx_bytes_per_sec: average_optional_f64(chunk.iter().map(|point| point.tx_bytes_per_sec)),
        latency_ms: average_optional_u64(chunk.iter().map(|point| point.latency_ms)),
        disk_used_percent: average_optional_f64(chunk.iter().map(|point| point.disk_used_percent)),
    }
}

fn average_f64(values: impl Iterator<Item = f64>) -> f64 {
    let mut total = 0.0;
    let mut count = 0_u64;
    for value in values {
        total += value;
        count += 1;
    }

    if count == 0 {
        0.0
    } else {
        total / count as f64
    }
}

fn average_optional_f64(values: impl Iterator<Item = Option<f64>>) -> Option<f64> {
    let mut total = 0.0;
    let mut count = 0_u64;
    for value in values.flatten() {
        total += value;
        count += 1;
    }

    (count > 0).then(|| total / count as f64)
}

fn average_optional_u64(values: impl Iterator<Item = Option<u64>>) -> Option<u64> {
    let mut total = 0_u128;
    let mut count = 0_u64;
    for value in values.flatten() {
        total += value as u128;
        count += 1;
    }

    (count > 0).then(|| (total / count as u128) as u64)
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use chrono::{Duration, Utc};
    use tokio::runtime::Runtime;
    use ximonitor_proto::{
        HistoryPoint, LoadAverage, MemoryUsage, NetworkCounters, NodeIdentity, NodeSnapshot,
        NodeStatus,
    };

    use super::{build_history_point, initialize_database, write_history_point};

    #[test]
    fn history_point_uses_server_last_seen_timestamp() {
        let now = Utc::now();
        let status = NodeStatus {
            identity: NodeIdentity {
                node_id: "hk-01".to_string(),
                node_label: "Hong Kong 01".to_string(),
                hostname: "hk-01.internal".to_string(),
                os: "Ubuntu".to_string(),
                kernel_version: None,
                cpu_model: None,
                cpu_cores: 2,
                agent_version: "0.1.0".to_string(),
                boot_time: None,
                tags: vec!["edge".to_string()],
            },
            snapshot: Some(NodeSnapshot {
                collected_at: now + Duration::hours(24),
                cpu_usage_percent: 42.0,
                load: LoadAverage {
                    one: 0.1,
                    five: 0.2,
                    fifteen: 0.3,
                },
                memory: MemoryUsage {
                    total_bytes: 1024,
                    used_bytes: 512,
                    available_bytes: 512,
                    swap_total_bytes: 0,
                    swap_used_bytes: 0,
                },
                uptime_secs: 60,
                disks: Vec::new(),
                network: NetworkCounters {
                    total_rx_bytes: 1,
                    total_tx_bytes: 2,
                    rx_bytes_per_sec: Some(3.0),
                    tx_bytes_per_sec: Some(4.0),
                },
            }),
            last_seen: Some(now),
            latency_ms: Some(12),
            online: true,
        };

        let point = build_history_point(&status).expect("history point should exist");
        assert_eq!(point.recorded_at, now);
    }

    #[test]
    #[cfg(unix)]
    fn history_database_artifacts_are_mode_600() {
        let runtime = Runtime::new().expect("runtime should build");
        runtime.block_on(async {
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock should be monotonic enough")
                .as_nanos();
            let temp_dir = std::env::temp_dir().join(format!("ximonitor-history-mode-{unique}"));
            std::fs::create_dir_all(&temp_dir).expect("temp dir should exist");
            let db_path = temp_dir.join("history.sqlite3");

            initialize_database(&db_path).expect("database should initialize");
            write_history_point(
                &db_path,
                &HistoryPoint {
                    node_id: "hk-01".to_string(),
                    recorded_at: Utc::now(),
                    cpu_usage_percent: 1.0,
                    memory_used_percent: 2.0,
                    rx_bytes_per_sec: Some(3.0),
                    tx_bytes_per_sec: Some(4.0),
                    latency_ms: Some(5),
                    disk_used_percent: Some(6.0),
                },
                None,
            )
            .expect("history point should persist");

            assert_mode_600(&db_path);
            for suffix in ["-wal", "-shm"] {
                let mut artifact = std::ffi::OsString::from(db_path.as_os_str());
                artifact.push(suffix);
                let artifact = std::path::PathBuf::from(artifact);
                if artifact.exists() {
                    assert_mode_600(&artifact);
                    let _ = std::fs::remove_file(&artifact);
                }
            }

            let _ = std::fs::remove_file(&db_path);
            let _ = std::fs::remove_dir(&temp_dir);
        });
    }

    #[cfg(unix)]
    fn assert_mode_600(path: &std::path::Path) {
        use std::os::unix::fs::PermissionsExt;

        let mode = std::fs::metadata(path)
            .expect("artifact metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
    }
}
