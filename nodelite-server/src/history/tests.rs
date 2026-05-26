//! Tests for history store writer, query, and throttling behavior.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::{Duration, Utc};
use nodelite_proto::{
    HistoryPoint, LoadAverage, MemoryUsage, NetworkCounters, NodeIdentity, NodeSnapshot,
    NodeStatus,
};
use tokio::runtime::Runtime;

use super::{
    HISTORY_CHANNEL_CAPACITY, HISTORY_QUERY_SQL, HistoryError, HistoryStore,
    SQLITE_BUSY_MAX_RETRIES, build_history_point, initialize_database, query_history_between,
    sqlite_busy_retry_delay, write_history_point,
};

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
        remote_ip: Some("198.51.100.24".to_string()),
        snapshot: Some(NodeSnapshot {
            collected_at: now + Duration::hours(24),
            cpu_usage_percent: Some(42.0),
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
        let temp_dir = std::env::temp_dir().join(format!("nodelite-history-mode-{unique}"));
        std::fs::create_dir_all(&temp_dir).expect("temp dir should exist");
        let data_dir = temp_dir.join("data");
        let db_path = data_dir.join("history.sqlite3");

        let mut connection = initialize_database(&db_path, 5).expect("database should initialize");
        write_history_point(
            &db_path,
            &mut connection,
            &HistoryPoint {
                node_id: "hk-01".to_string(),
                recorded_at: Utc::now(),
                cpu_usage_percent: Some(1.0),
                memory_used_percent: 2.0,
                rx_bytes_per_sec: Some(3.0),
                tx_bytes_per_sec: Some(4.0),
                latency_ms: Some(5),
                disk_used_percent: Some(6.0),
            },
            None,
            &AtomicBool::new(false),
        )
        .expect("history point should persist");

        assert_mode_700(&data_dir);
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
        let _ = std::fs::remove_dir(&data_dir);
        let _ = std::fs::remove_dir(&temp_dir);
    });
}

#[test]
fn forget_missing_prunes_retired_nodes_from_write_throttle_state() {
    let runtime = Runtime::new().expect("runtime should build");
    runtime.block_on(async {
        let store = HistoryStore::new(PathBuf::from("./data/history.sqlite3"), 5);
        {
            let mut guard = store.last_written_at.lock().await;
            guard.insert("hk-01".to_string(), Utc::now());
            guard.insert("jp-01".to_string(), Utc::now());
            guard.insert("us-01".to_string(), Utc::now());
        }

        let removed = store
            .forget_missing(&["jp-01".to_string(), "us-01".to_string()])
            .await;
        assert_eq!(removed, 1);

        let guard = store.last_written_at.lock().await;
        assert!(!guard.contains_key("hk-01"));
        assert!(guard.contains_key("jp-01"));
        assert!(guard.contains_key("us-01"));
    });
}

#[tokio::test]
async fn query_history_reports_connection_not_initialized() {
    let store = HistoryStore::new(PathBuf::from("./data/history.sqlite3"), 5);
    store.available.store(true, Ordering::Relaxed);

    let error = store
        .query_history("hk-01", 1, 60)
        .await
        .expect_err("query should surface typed connection error");

    assert!(matches!(error, HistoryError::ConnectionNotInitialized));
}

#[tokio::test]
async fn query_history_range_reports_connection_not_initialized() {
    let store = HistoryStore::new(PathBuf::from("./data/history.sqlite3"), 5);
    store.available.store(true, Ordering::Relaxed);

    let now = Utc::now();
    let error = store
        .query_history_range("hk-01", now - Duration::hours(1), now, 60)
        .await
        .expect_err("range query should surface typed connection error");

    assert!(matches!(error, HistoryError::ConnectionNotInitialized));
}

#[test]
fn query_history_between_buckets_and_limits_results() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be monotonic enough")
        .as_nanos();
    let temp_dir = std::env::temp_dir().join(format!("nodelite-history-query-{unique}"));
    std::fs::create_dir_all(&temp_dir).expect("temp dir should exist");
    let db_path = temp_dir.join("history.sqlite3");
    let mut connection = initialize_database(&db_path, 5).expect("database should initialize");
    let hardened = AtomicBool::new(false);
    let start = Utc::now() - Duration::hours(6);
    for index in 0..180 {
        write_history_point(
            &db_path,
            &mut connection,
            &HistoryPoint {
                node_id: "hk-01".to_string(),
                recorded_at: start + Duration::seconds(index * 120),
                cpu_usage_percent: Some(index as f64),
                memory_used_percent: 50.0,
                rx_bytes_per_sec: Some(index as f64),
                tx_bytes_per_sec: Some(index as f64 / 2.0),
                latency_ms: Some((index % 10) as u64),
                disk_used_percent: Some(60.0),
            },
            None,
            &hardened,
        )
        .expect("history point should persist");
    }

    let points = query_history_between(&connection, "hk-01", start, Utc::now(), 24)
        .expect("history query should succeed");
    assert!(!points.is_empty());
    assert!(points.len() <= 24);
    assert!(
        points
            .windows(2)
            .all(|pair| pair[0].recorded_at <= pair[1].recorded_at)
    );

    let _ = std::fs::remove_file(&db_path);
    let _ = std::fs::remove_dir(&temp_dir);
}

#[test]
fn history_accepts_unknown_cpu_usage_after_schema_migration() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be monotonic enough")
        .as_nanos();
    let temp_dir = std::env::temp_dir().join(format!("nodelite-history-null-cpu-{unique}"));
    std::fs::create_dir_all(&temp_dir).expect("temp dir should exist");
    let db_path = temp_dir.join("history.sqlite3");
    {
        let connection = rusqlite::Connection::open(&db_path).expect("legacy database should open");
        connection
            .execute_batch(
                r#"
                CREATE TABLE history_points (
                    node_id TEXT NOT NULL,
                    recorded_at INTEGER NOT NULL,
                    cpu_usage_percent REAL NOT NULL,
                    memory_used_percent REAL NOT NULL,
                    rx_bytes_per_sec REAL,
                    tx_bytes_per_sec REAL,
                    latency_ms INTEGER,
                    disk_used_percent REAL
                );
                CREATE INDEX idx_history_points_node_time
                    ON history_points (node_id, recorded_at);
                CREATE INDEX idx_history_points_covering_metrics
                    ON history_points (
                        node_id,
                        recorded_at,
                        cpu_usage_percent,
                        memory_used_percent,
                        rx_bytes_per_sec,
                        tx_bytes_per_sec,
                        latency_ms,
                        disk_used_percent
                    );
                "#,
            )
            .expect("legacy schema should be created");
    }

    let mut connection = initialize_database(&db_path, 5).expect("database should migrate");
    let cpu_not_null: i64 = connection
        .query_row(
            "SELECT [notnull] FROM pragma_table_info('history_points') WHERE name = 'cpu_usage_percent'",
            [],
            |row| row.get(0),
        )
        .expect("cpu column metadata should be readable");
    assert_eq!(cpu_not_null, 0);

    let recorded_at = Utc::now();
    write_history_point(
        &db_path,
        &mut connection,
        &HistoryPoint {
            node_id: "hk-01".to_string(),
            recorded_at,
            cpu_usage_percent: None,
            memory_used_percent: 50.0,
            rx_bytes_per_sec: None,
            tx_bytes_per_sec: None,
            latency_ms: None,
            disk_used_percent: None,
        },
        None,
        &AtomicBool::new(false),
    )
    .expect("unknown cpu history point should persist");

    let points = query_history_between(
        &connection,
        "hk-01",
        recorded_at - Duration::seconds(1),
        recorded_at + Duration::seconds(1),
        60,
    )
    .expect("history query should succeed");
    assert_eq!(points.len(), 1);
    assert_eq!(points[0].cpu_usage_percent, None);

    let _ = std::fs::remove_file(&db_path);
    let _ = std::fs::remove_dir(&temp_dir);
}

#[test]
fn query_history_between_uses_covering_index() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be monotonic enough")
        .as_nanos();
    let temp_dir = std::env::temp_dir().join(format!("nodelite-history-query-plan-{unique}"));
    std::fs::create_dir_all(&temp_dir).expect("temp dir should exist");
    let db_path = temp_dir.join("history.sqlite3");
    let connection = initialize_database(&db_path, 5).expect("database should initialize");
    let explain_sql = format!("EXPLAIN QUERY PLAN {HISTORY_QUERY_SQL}");
    let mut statement = connection
        .prepare(&explain_sql)
        .expect("query plan should prepare");
    let details = statement
        .query_map(
            rusqlite::params!["hk-01", 0_i64, i64::MAX, 60_i64, 24_i64],
            |row| row.get::<_, String>(3),
        )
        .expect("query plan should run")
        .collect::<Result<Vec<_>, _>>()
        .expect("query plan rows should decode");
    let plan = details.join("\n");

    assert!(
        plan.contains("USING COVERING INDEX idx_history_points_covering_metrics"),
        "history query should use covering index, got:\n{plan}"
    );

    drop(statement);
    drop(connection);
    let _ = std::fs::remove_file(&db_path);
    let _ = std::fs::remove_dir(&temp_dir);
}

#[test]
fn sqlite_busy_retry_delay_uses_capped_exponential_backoff() {
    let delays_ms = (1..=8)
        .map(|attempt| sqlite_busy_retry_delay(attempt).as_millis())
        .collect::<Vec<_>>();

    assert_eq!(SQLITE_BUSY_MAX_RETRIES, 10);
    assert_eq!(delays_ms, vec![50, 100, 200, 400, 800, 1000, 1000, 1000]);
}

fn fake_status_for(node_id: &str, recorded_at: chrono::DateTime<Utc>) -> NodeStatus {
    NodeStatus {
        identity: NodeIdentity {
            node_id: node_id.to_string(),
            node_label: format!("{node_id}-label"),
            hostname: format!("{node_id}.internal"),
            os: "Ubuntu".to_string(),
            kernel_version: None,
            cpu_model: None,
            cpu_cores: 2,
            agent_version: "0.1.0".to_string(),
            boot_time: None,
            tags: Vec::new(),
        },
        remote_ip: Some("198.51.100.24".to_string()),
        snapshot: Some(NodeSnapshot {
            collected_at: recorded_at,
            cpu_usage_percent: Some(42.0),
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
        last_seen: Some(recorded_at),
        latency_ms: Some(12),
        online: true,
    }
}

fn temp_history_db_path(test_name: &str) -> std::path::PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be monotonic enough")
        .as_nanos();
    let temp_dir = std::env::temp_dir().join(format!("nodelite-history-{test_name}-{unique}"));
    std::fs::create_dir_all(&temp_dir).expect("temp dir should exist");
    temp_dir.join("history.sqlite3")
}

/// 集成 record_status -> writer task -> SQLite 全链路:
/// 多次写入要在 channel + batch 模型下全部落库,而不仅仅是最后一条。
#[tokio::test]
async fn record_status_flushes_through_writer_task_to_sqlite() {
    let db_path = temp_history_db_path("writer-task");
    let store = HistoryStore::new(db_path.clone(), 5);
    store.initialize().await;
    assert!(store.is_available());

    // 写入 5 个不同节点的样本(同节点会被 throttle 拦掉,所以这里用不同 node_id)。
    let now = Utc::now();
    for i in 0..5 {
        let node_id = format!("node-{i:02}");
        let status = fake_status_for(&node_id, now);
        store.record_status(&status).await;
    }

    // 触发 shutdown; writer 会把已经入队但还没 flush 的样本 drain 出来。
    store.shutdown().await;
    assert_eq!(store.dropped_writes(), 0, "no writes should have been dropped");

    // 验证 5 条样本都成功落库。
    let connection = initialize_database(&db_path, 5).expect("re-open database");
    let count: i64 = connection
        .query_row("SELECT COUNT(*) FROM history_points", [], |row| row.get(0))
        .expect("count query");
    assert_eq!(count, 5);

    let _ = std::fs::remove_file(&db_path);
    if let Some(parent) = db_path.parent() {
        let _ = std::fs::remove_dir(parent);
    }
}

#[tokio::test]
async fn query_history_does_not_wait_for_write_connection_lock() {
    let db_path = temp_history_db_path("query-read-connection");
    let store = HistoryStore::new(db_path.clone(), 5);
    store.initialize().await;
    assert!(store.is_available());

    let status = fake_status_for("hk-01", Utc::now());
    store.record_status(&status).await;
    store.shutdown().await;

    let write_guard = store.write_connection.lock().await;
    let points = tokio::time::timeout(
        std::time::Duration::from_secs(1),
        store.query_history("hk-01", 1, 60),
    )
    .await
    .expect("query should not wait for write connection lock")
    .expect("query should succeed through read connection");
    drop(write_guard);

    assert!(!points.is_empty());

    let _ = std::fs::remove_file(&db_path);
    if let Some(parent) = db_path.parent() {
        let _ = std::fs::remove_dir(parent);
    }
}

#[tokio::test]
async fn history_writer_does_not_wait_for_read_connection_lock() {
    let db_path = temp_history_db_path("writer-write-connection");
    let store = HistoryStore::new(db_path.clone(), 5);
    store.initialize().await;
    assert!(store.is_available());

    let read_guard = store.read_connection.lock().await;
    let status = fake_status_for("hk-01", Utc::now());
    store.record_status(&status).await;
    tokio::time::timeout(std::time::Duration::from_secs(1), store.shutdown())
        .await
        .expect("writer flush should not wait for read connection lock");
    drop(read_guard);

    let connection = initialize_database(&db_path, 5).expect("re-open database");
    let count: i64 = connection
        .query_row("SELECT COUNT(*) FROM history_points", [], |row| row.get(0))
        .expect("count query");
    assert_eq!(count, 1);

    let _ = std::fs::remove_file(&db_path);
    if let Some(parent) = db_path.parent() {
        let _ = std::fs::remove_dir(parent);
    }
}

#[tokio::test]
async fn record_status_does_not_throttle_after_queue_full_drop() {
    let db_path = temp_history_db_path("queue-full-throttle");
    let store = HistoryStore::new(db_path.clone(), 5);
    store.available.store(true, Ordering::Relaxed);
    let (tx, _rx) = tokio::sync::mpsc::channel::<HistoryPoint>(HISTORY_CHANNEL_CAPACITY);
    for index in 0..HISTORY_CHANNEL_CAPACITY {
        tx.try_send(HistoryPoint {
            node_id: format!("queued-{index}"),
            recorded_at: Utc::now(),
            cpu_usage_percent: Some(1.0),
            memory_used_percent: 2.0,
            rx_bytes_per_sec: Some(3.0),
            tx_bytes_per_sec: Some(4.0),
            latency_ms: Some(5),
            disk_used_percent: Some(6.0),
        })
        .expect("test channel should accept prefilled point");
    }
    {
        let mut guard = store.writer_tx.write().await;
        *guard = Some(tx);
    }

    let status = fake_status_for("hk-01", Utc::now());
    store.record_status(&status).await;

    assert_eq!(store.dropped_writes(), 1);
    let guard = store.last_written_at.lock().await;
    assert!(
        !guard.contains_key("hk-01"),
        "dropped writes must not advance the throttle window"
    );

    let _ = std::fs::remove_file(&db_path);
    if let Some(parent) = db_path.parent() {
        let _ = std::fs::remove_dir(parent);
    }
}

#[tokio::test]
async fn record_status_skips_point_build_when_throttled() {
    let db_path = temp_history_db_path("throttled-builder");
    let store = HistoryStore::new(db_path.clone(), 5);
    store.available.store(true, Ordering::Relaxed);
    let (tx, _rx) = tokio::sync::mpsc::channel::<HistoryPoint>(1);
    {
        let mut guard = store.writer_tx.write().await;
        *guard = Some(tx);
    }

    let now = Utc::now();
    {
        let mut guard = store.last_written_at.lock().await;
        guard.insert("hk-01".to_string(), now);
    }

    let builds = AtomicUsize::new(0);
    let status = fake_status_for("hk-01", now);
    store
        .record_status_with_builder(&status, |_| {
            builds.fetch_add(1, Ordering::Relaxed);
            build_history_point(&status)
        })
        .await;

    assert_eq!(
        builds.load(Ordering::Relaxed),
        0,
        "throttled samples should return before building a HistoryPoint"
    );

    let _ = std::fs::remove_file(&db_path);
    if let Some(parent) = db_path.parent() {
        let _ = std::fs::remove_dir(parent);
    }
}

/// shutdown() 之后再调用 record_status 必须立刻返回,
/// 不能 panic 也不能阻塞;此时 sender 已被清空,store 会进入 unavailable 状态。
#[tokio::test]
async fn record_status_is_noop_after_shutdown() {
    let db_path = temp_history_db_path("after-shutdown");
    let store = HistoryStore::new(db_path.clone(), 5);
    store.initialize().await;
    store.shutdown().await;

    // shutdown 不会触发 dropped 计数;它走的是 writer_tx 被 take 走的快速 return 路径。
    let status = fake_status_for("hk-01", Utc::now());
    store.record_status(&status).await;
    assert_eq!(store.dropped_writes(), 0);

    let _ = std::fs::remove_file(&db_path);
    if let Some(parent) = db_path.parent() {
        let _ = std::fs::remove_dir(parent);
    }
}

#[cfg(unix)]
fn assert_mode_700(path: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt;

    let mode = std::fs::metadata(path)
        .expect("artifact metadata")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o700);
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
