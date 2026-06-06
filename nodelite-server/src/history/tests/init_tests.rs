use super::*;

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

#[tokio::test]
async fn query_history_reports_query_error_when_read_database_is_missing() {
    let db_path = temp_history_db_path("missing-query-db");
    let store = HistoryStore::new(db_path.clone(), 5);
    store.available.store(true, Ordering::Relaxed);

    let error = store
        .query_history("hk-01", 1, 60)
        .await
        .expect_err("query should surface read connection error");

    assert!(matches!(error, HistoryError::Query(_)));

    if let Some(parent) = db_path.parent() {
        let _ = std::fs::remove_dir(parent);
    }
}

#[tokio::test]
async fn query_history_range_reports_query_error_when_read_database_is_missing() {
    let db_path = temp_history_db_path("missing-range-query-db");
    let store = HistoryStore::new(db_path.clone(), 5);
    store.available.store(true, Ordering::Relaxed);

    let now = Utc::now();
    let error = store
        .query_history_range("hk-01", now - Duration::hours(1), now, 60)
        .await
        .expect_err("range query should surface read connection error");

    assert!(matches!(error, HistoryError::Query(_)));

    if let Some(parent) = db_path.parent() {
        let _ = std::fs::remove_dir(parent);
    }
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
fn sqlite_busy_retry_delay_uses_capped_exponential_backoff() {
    let delays_ms = (1..=8)
        .map(|attempt| sqlite_busy_retry_delay(attempt).as_millis())
        .collect::<Vec<_>>();

    assert_eq!(SQLITE_BUSY_MAX_RETRIES, 10);
    assert_eq!(delays_ms, vec![50, 100, 200, 400, 800, 1000, 1000, 1000]);
}
