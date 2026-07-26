use super::*;

fn persisted_history_point_count(db_path: &PathBuf) -> i64 {
    let connection = rusqlite::Connection::open(db_path).expect("history database should open");
    connection
        .query_row("SELECT COUNT(*) FROM history_points", [], |row| row.get(0))
        .expect("history count query should succeed")
}

async fn wait_for_persisted_history_point_count(db_path: &PathBuf, expected: i64) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
    loop {
        let actual = persisted_history_point_count(db_path);
        if actual == expected {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "expected {expected} persisted history points, found {actual}"
        );
        tokio::task::yield_now().await;
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
}

#[test]
fn forget_missing_prunes_retired_nodes_from_write_throttle_state() {
    let runtime = Runtime::new().expect("runtime should build");
    runtime.block_on(async {
        let store = test_history_store(PathBuf::from("./data/history.sqlite3"));
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
async fn record_status_flushes_through_writer_task_to_sqlite() {
    let db_path = temp_history_db_path("writer-task");
    let store = test_history_store(db_path.clone());
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
    assert_eq!(
        store.dropped_writes(),
        0,
        "no writes should have been dropped"
    );

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

#[tokio::test(start_paused = true)]
async fn history_writer_flushes_when_configured_batch_max_is_reached() {
    let db_path = temp_history_db_path("writer-configured-batch");
    let store = HistoryStore::new_with_writer_schedule(
        db_path.clone(),
        5,
        DEFAULT_HISTORY_QUERY_CONCURRENCY,
        DEFAULT_HISTORY_READ_CACHE_KIB,
        2,
        std::time::Duration::from_secs(60),
    );
    store.initialize().await;
    assert!(store.is_available());
    tokio::task::yield_now().await;

    let paused_at = tokio::time::Instant::now();
    store
        .record_status(&fake_status_for("batch-node-01", Utc::now()))
        .await;
    tokio::task::yield_now().await;
    assert_eq!(persisted_history_point_count(&db_path), 0);

    store
        .record_status(&fake_status_for("batch-node-02", Utc::now()))
        .await;
    wait_for_persisted_history_point_count(&db_path, 2).await;
    assert_eq!(tokio::time::Instant::now(), paused_at);

    store.shutdown().await;
    let _ = std::fs::remove_file(&db_path);
    if let Some(parent) = db_path.parent() {
        let _ = std::fs::remove_dir(parent);
    }
}

#[tokio::test(start_paused = true)]
async fn history_writer_flushes_at_configured_interval() {
    let db_path = temp_history_db_path("writer-configured-interval");
    let store = HistoryStore::new_with_writer_schedule(
        db_path.clone(),
        5,
        DEFAULT_HISTORY_QUERY_CONCURRENCY,
        DEFAULT_HISTORY_READ_CACHE_KIB,
        128,
        std::time::Duration::from_millis(10),
    );
    store.initialize().await;
    assert!(store.is_available());
    tokio::task::yield_now().await;

    store
        .record_status(&fake_status_for("interval-node", Utc::now()))
        .await;
    tokio::task::yield_now().await;
    assert_eq!(persisted_history_point_count(&db_path), 0);

    let before_advance = tokio::time::Instant::now();
    tokio::time::advance(std::time::Duration::from_millis(10)).await;
    wait_for_persisted_history_point_count(&db_path, 1).await;
    assert_eq!(
        tokio::time::Instant::now(),
        before_advance + std::time::Duration::from_millis(10)
    );

    store.shutdown().await;
    let _ = std::fs::remove_file(&db_path);
    if let Some(parent) = db_path.parent() {
        let _ = std::fs::remove_dir(parent);
    }
}

#[tokio::test]
async fn record_status_does_not_throttle_after_queue_full_drop() {
    let db_path = temp_history_db_path("queue-full-throttle");
    let store = test_history_store(db_path.clone());
    store.available.store(true, Ordering::Relaxed);
    let (tx, _rx) = tokio::sync::mpsc::channel::<HistoryPoint>(HISTORY_CHANNEL_CAPACITY);
    for index in 0..HISTORY_CHANNEL_CAPACITY {
        tx.try_send(HistoryPoint {
            node_id: format!("queued-{index}"),
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
    let store = test_history_store(db_path.clone());
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

#[tokio::test]
async fn record_status_is_noop_after_shutdown() {
    let db_path = temp_history_db_path("after-shutdown");
    let store = test_history_store(db_path.clone());
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
