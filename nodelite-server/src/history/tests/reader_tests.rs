use super::*;

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
        geoip_country: None,
        geoip_city: None,
        geoip_latitude: None,
        geoip_longitude: None,
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
async fn concurrent_history_queries_use_independent_read_connections() {
    let db_path = temp_history_db_path("query-concurrent-readers");
    let mut connection = initialize_database(&db_path, 5).expect("database should initialize");
    let hardened = AtomicBool::new(false);
    let start = Utc::now() - Duration::hours(2);
    for index in 0..240 {
        write_history_point(
            &db_path,
            &mut connection,
            &HistoryPoint {
                node_id: "hk-01".to_string(),
                recorded_at: start + Duration::seconds(index * 30),
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
    drop(connection);

    let store = HistoryStore::new(db_path.clone(), 5);
    store.initialize().await;
    assert!(store.is_available());

    let end = start + Duration::seconds(240 * 30);
    let tasks = (0..8)
        .map(|_| {
            let store = store.clone();
            tokio::spawn(async move { store.query_history_range("hk-01", start, end, 120).await })
        })
        .collect::<Vec<_>>();

    for task in tasks {
        let points = task
            .await
            .expect("query task should not panic")
            .expect("history query should succeed");
        assert!(!points.is_empty());
        assert!(points.len() <= 120);
    }
    store.shutdown().await;

    let _ = std::fs::remove_file(&db_path);
    if let Some(parent) = db_path.parent() {
        let _ = std::fs::remove_dir(parent);
    }
}
