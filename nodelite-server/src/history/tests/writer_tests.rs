use super::*;

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
