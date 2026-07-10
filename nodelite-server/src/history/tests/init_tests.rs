use super::*;

#[test]
fn read_connection_uses_bounded_private_page_cache() {
    let db_path = temp_history_db_path("read-cache-size");
    let write_connection = initialize_database(&db_path, 5).expect("database should initialize");
    drop(write_connection);

    let read_connection = open_read_connection(&db_path, 5, Some(DEFAULT_HISTORY_READ_CACHE_KIB))
        .expect("read connection should initialize");
    let cache_size: i64 = read_connection
        .query_row("PRAGMA cache_size", [], |row| row.get(0))
        .expect("cache size should be readable");
    assert_eq!(cache_size, -(DEFAULT_HISTORY_READ_CACHE_KIB as i64));

    drop(read_connection);
    let _ = std::fs::remove_file(&db_path);
    if let Some(parent) = db_path.parent() {
        let _ = std::fs::remove_dir(parent);
    }
}

#[test]
fn read_connection_can_preserve_sqlite_default_page_cache() {
    let db_path = temp_history_db_path("default-read-cache-size");
    let write_connection = initialize_database(&db_path, 5).expect("database should initialize");
    drop(write_connection);

    let expected: i64 =
        rusqlite::Connection::open_with_flags(&db_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .expect("reference read connection should initialize")
            .query_row("PRAGMA cache_size", [], |row| row.get(0))
            .expect("reference cache size should be readable");
    let read_connection =
        open_read_connection(&db_path, 5, None).expect("read connection should initialize");
    let cache_size: i64 = read_connection
        .query_row("PRAGMA cache_size", [], |row| row.get(0))
        .expect("cache size should be readable");

    assert_eq!(cache_size, expected);

    drop(read_connection);
    let _ = std::fs::remove_file(&db_path);
    if let Some(parent) = db_path.parent() {
        let _ = std::fs::remove_dir(parent);
    }
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
                load_one: Some(1.1),
                load_five: Some(1.2),
                load_fifteen: Some(1.3),
                memory_used_percent: 2.0,
                rx_bytes_per_sec: Some(3.0),
                tx_bytes_per_sec: Some(4.0),
                latency_ms: Some(5),
                packet_loss_percent: Some(0.5),
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
    let store = test_history_store(db_path.clone());
    store.available.store(true, Ordering::Relaxed);

    let error = store
        .query_history("hk-01", 1, 60)
        .await
        .expect_err("query should surface read connection error");

    assert!(matches!(error, HistoryError::Query(_)));
    let metrics = store.query_runtime_metrics();
    assert_eq!(metrics.permits_in_use, 0);
    assert_eq!(metrics.waiting, 0);
    assert_eq!(metrics.acquisitions_total, 1);

    if let Some(parent) = db_path.parent() {
        let _ = std::fs::remove_dir(parent);
    }
}

#[tokio::test]
async fn query_history_range_reports_query_error_when_read_database_is_missing() {
    let db_path = temp_history_db_path("missing-range-query-db");
    let store = test_history_store(db_path.clone());
    store.available.store(true, Ordering::Relaxed);

    let now = Utc::now();
    let error = store
        .query_history_range("hk-01", now - Duration::hours(1), now, 60)
        .await
        .expect_err("range query should surface read connection error");

    assert!(matches!(error, HistoryError::Query(_)));
    let metrics = store.query_runtime_metrics();
    assert_eq!(metrics.permits_in_use, 0);
    assert_eq!(metrics.waiting, 0);
    assert_eq!(metrics.acquisitions_total, 1);

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
    for column in [
        "load_one",
        "load_five",
        "load_fifteen",
        "packet_loss_percent",
    ] {
        let count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('history_points') WHERE name = ?1",
                [column],
                |row| row.get(0),
            )
            .expect("load column metadata should be readable");
        assert_eq!(count, 1);
    }

    let recorded_at = Utc::now();
    write_history_point(
        &db_path,
        &mut connection,
        &HistoryPoint {
            node_id: "hk-01".to_string(),
            recorded_at,
            cpu_usage_percent: None,
            load_one: Some(0.4),
            load_five: Some(0.5),
            load_fifteen: Some(0.6),
            memory_used_percent: 50.0,
            rx_bytes_per_sec: None,
            tx_bytes_per_sec: None,
            latency_ms: None,
            packet_loss_percent: Some(0.25),
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
    assert_eq!(points[0].load_one, Some(0.4));
    assert_eq!(points[0].load_five, Some(0.5));
    assert_eq!(points[0].load_fifteen, Some(0.6));
    assert_eq!(points[0].packet_loss_percent, Some(0.25));

    let _ = std::fs::remove_file(&db_path);
    let _ = std::fs::remove_dir(&temp_dir);
}

#[test]
fn history_adds_load_columns_to_existing_schema() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be monotonic enough")
        .as_nanos();
    let temp_dir = std::env::temp_dir().join(format!("nodelite-history-load-migration-{unique}"));
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
                    cpu_usage_percent REAL,
                    memory_used_percent REAL NOT NULL,
                    rx_bytes_per_sec REAL,
                    tx_bytes_per_sec REAL,
                    latency_ms INTEGER,
                    disk_used_percent REAL
                );
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

    let connection = initialize_database(&db_path, 5).expect("database should migrate");
    for column in [
        "load_one",
        "load_five",
        "load_fifteen",
        "packet_loss_percent",
    ] {
        let count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('history_points') WHERE name = ?1",
                [column],
                |row| row.get(0),
            )
            .expect("load column metadata should be readable");
        assert_eq!(count, 1);
    }
    let index_columns = connection
        .prepare("PRAGMA index_info(idx_history_points_covering_metrics)")
        .expect("covering index metadata should prepare")
        .query_map([], |row| row.get::<_, String>(2))
        .expect("covering index metadata should query")
        .collect::<Result<Vec<_>, _>>()
        .expect("covering index columns should decode");
    assert!(index_columns.iter().any(|column| column == "load_one"));
    assert!(index_columns.iter().any(|column| column == "load_five"));
    assert!(index_columns.iter().any(|column| column == "load_fifteen"));

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
