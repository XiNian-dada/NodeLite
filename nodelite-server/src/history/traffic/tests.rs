//! 套餐账本的计数、持久化与恢复测试。

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use chrono::{TimeZone, Utc};

use super::{
    TrafficFlushBarrier, TrafficLedgerHealth, TrafficPersistenceError, TrafficSample,
    TrafficTracker, counted_bytes, run_traffic_writer, utc_month_start,
};
use crate::registry::TrafficAccounting;

const BIDI: TrafficAccounting = TrafficAccounting::Bidirectional;

fn test_db_path(name: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time should be available")
        .as_nanos();
    std::env::temp_dir().join(format!("nodelite-traffic-{name}-{unique}.sqlite3"))
}

fn remove_db(db_path: &Path) {
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{suffix}", db_path.display()));
    }
}

async fn started_tracker(db_path: &Path) -> TrafficTracker {
    let tracker = TrafficTracker::new(Arc::new(db_path.to_path_buf()), 0);
    tracker
        .initialize()
        .await
        .expect("tracker should initialize");
    tracker
}

fn sample(rx_bytes: u64, tx_bytes: u64, uptime_secs: u64) -> TrafficSample {
    TrafficSample {
        rx_bytes,
        tx_bytes,
        uptime_secs,
    }
}

async fn used(tracker: &TrafficTracker, frame: TrafficSample) -> u64 {
    tracker
        .record("node-a", frame, BIDI, true)
        .await
        .expect("ledger should be available")
        .expect("enabled quota should return usage")
        .used_bytes
}

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
    let db_path = test_db_path("deltas");
    let tracker = started_tracker(&db_path).await;

    assert_eq!(used(&tracker, sample(100, 200, 10)).await, 0);
    assert_eq!(used(&tracker, sample(130, 260, 20)).await, 90);

    let reset = tracker
        .record(
            "node-a",
            sample(140, 290, 30),
            TrafficAccounting::Outbound,
            true,
        )
        .await
        .expect("ledger should be available")
        .expect("changed accounting should return usage");
    assert_eq!(reset.used_bytes, 0);
    let outbound = tracker
        .record(
            "node-a",
            sample(160, 330, 40),
            TrafficAccounting::Outbound,
            true,
        )
        .await
        .expect("ledger should be available")
        .expect("outbound usage should be recorded");
    assert_eq!(outbound.used_bytes, 40);

    tracker.shutdown().await;
    remove_db(&db_path);
}

#[tokio::test]
async fn transient_counter_drop_keeps_monthly_usage_unchanged() {
    const TWO_TB: u64 = 2_000_000_000_000;
    let db_path = test_db_path("transient-drop");
    let tracker = started_tracker(&db_path).await;

    assert_eq!(used(&tracker, sample(0, 0, 100)).await, 0);
    assert_eq!(used(&tracker, sample(TWO_TB, 0, 110)).await, TWO_TB);
    assert_eq!(used(&tracker, sample(0, 0, 120)).await, TWO_TB);
    assert_eq!(used(&tracker, sample(TWO_TB, 0, 130)).await, TWO_TB);

    tracker.shutdown().await;
    remove_db(&db_path);
}

#[tokio::test]
async fn reboot_detection_survives_a_server_restart() {
    let db_path = test_db_path("restart-uptime");
    let tracker = started_tracker(&db_path).await;
    used(&tracker, sample(1_000, 0, 500)).await;
    assert_eq!(used(&tracker, sample(1_500, 0, 510)).await, 500);
    tracker.shutdown().await;

    let restarted = started_tracker(&db_path).await;
    assert_eq!(restarted.usages().await[0].used_bytes, 500);
    // 同一轮 uptime 内的回退仍被保持：若 uptime 没落盘，这里会被误判为重启。
    assert_eq!(used(&restarted, sample(0, 0, 520)).await, 500);
    assert_eq!(used(&restarted, sample(1_600, 0, 530)).await, 600);
    assert_eq!(used(&restarted, sample(10, 0, 5)).await, 600);
    assert_eq!(used(&restarted, sample(30, 0, 15)).await, 620);

    restarted.shutdown().await;
    remove_db(&db_path);
}

#[tokio::test]
async fn legacy_ledger_rows_are_migrated_and_keep_their_usage() {
    let db_path = test_db_path("legacy");
    let connection = rusqlite::Connection::open(&db_path).expect("db should open");
    connection
        .execute_batch(
            "CREATE TABLE traffic_usage (node_id TEXT PRIMARY KEY NOT NULL, cycle_started_at INTEGER NOT NULL, accounting TEXT NOT NULL, used_bytes INTEGER NOT NULL, last_rx_bytes INTEGER NOT NULL, last_tx_bytes INTEGER NOT NULL);",
        )
        .expect("legacy schema should apply");
    connection
        .execute(
            "INSERT INTO traffic_usage VALUES ('node-a', ?1, 'bidirectional', 700, 900, 100)",
            [utc_month_start(Utc::now()).timestamp()],
        )
        .expect("legacy row should insert");
    drop(connection);

    let tracker = started_tracker(&db_path).await;
    assert_eq!(tracker.usages().await[0].used_bytes, 700);
    assert_eq!(used(&tracker, sample(5, 5, 1_000)).await, 700);
    assert_eq!(used(&tracker, sample(15, 5, 1_010)).await, 710);

    tracker.shutdown().await;
    remove_db(&db_path);
}

#[tokio::test]
async fn record_reports_unavailable_when_the_ledger_never_loaded() {
    let db_path = test_db_path("never-loaded");
    let tracker = TrafficTracker::new(Arc::new(db_path.clone()), 0);
    let result = tracker.record("node-a", sample(1, 1, 1), BIDI, true).await;
    assert!(matches!(result, Err(TrafficPersistenceError::Unavailable)));
    assert!(tracker.usages().await.is_empty());
    remove_db(&db_path);
}

fn lock_database(db_path: &Path) -> rusqlite::Connection {
    let connection = rusqlite::Connection::open(db_path).expect("db should open");
    connection
        .execute_batch("BEGIN EXCLUSIVE;")
        .expect("exclusive lock should be acquired");
    connection
}

fn persisted_usage(db_path: &Path) -> Option<i64> {
    rusqlite::Connection::open(db_path)
        .expect("db should open")
        .query_row(
            "SELECT used_bytes FROM traffic_usage WHERE node_id = 'node-a'",
            [],
            |row| row.get(0),
        )
        .ok()
}

async fn wait_for_health(
    tracker: &TrafficTracker,
    condition: impl Fn(TrafficLedgerHealth) -> bool,
) -> TrafficLedgerHealth {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let health = tracker.health();
            if condition(health) {
                return health;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("health condition should be reached")
}

#[tokio::test]
async fn failed_writes_are_observable_retried_and_recovered_after_restart() {
    let db_path = test_db_path("write-failure");
    let tracker = started_tracker(&db_path).await;
    assert!(tracker.health().available);
    used(&tracker, sample(100, 0, 10)).await;
    wait_for_health(&tracker, |health| health.last_success_at > 0).await;

    let lock = lock_database(&db_path);
    assert_eq!(used(&tracker, sample(400, 0, 20)).await, 300);
    let degraded = wait_for_health(&tracker, |health| health.write_degraded).await;
    assert!(degraded.write_failures >= 1);
    assert_eq!(degraded.pending_writes, 1);
    // 重试期间新的累计值覆盖旧值，而不是叠加。
    assert_eq!(used(&tracker, sample(500, 0, 30)).await, 400);

    drop(lock);
    wait_for_health(&tracker, |health| {
        !health.write_degraded && health.pending_writes == 0
    })
    .await;
    assert_eq!(persisted_usage(&db_path), Some(400));
    tracker.shutdown().await;

    let restarted = started_tracker(&db_path).await;
    assert_eq!(restarted.usages().await[0].used_bytes, 400);
    assert_eq!(used(&restarted, sample(510, 0, 40)).await, 410);
    restarted.shutdown().await;
    remove_db(&db_path);
}

#[tokio::test]
async fn removal_supersedes_a_retained_upsert() {
    let db_path = test_db_path("retained-remove");
    let tracker = started_tracker(&db_path).await;
    used(&tracker, sample(100, 0, 10)).await;
    wait_for_health(&tracker, |health| health.last_success_at > 0).await;

    let lock = lock_database(&db_path);
    used(&tracker, sample(400, 0, 20)).await;
    wait_for_health(&tracker, |health| health.write_degraded).await;
    assert!(
        tracker
            .remove_nodes(vec!["node-a".to_string()])
            .await
            .is_err()
    );
    drop(lock);
    wait_for_health(&tracker, |health| {
        health.pending_writes == 0 && !health.write_degraded
    })
    .await;
    assert_eq!(persisted_usage(&db_path), None);

    tracker.shutdown().await;
    let restarted = started_tracker(&db_path).await;
    assert!(restarted.usages().await.is_empty());
    restarted.shutdown().await;
    remove_db(&db_path);
}

#[tokio::test]
async fn removal_does_not_hold_state_while_waiting_for_a_full_queue() {
    let db_path = test_db_path("remove-full-queue");
    let tracker = started_tracker(&db_path).await;
    tracker.shutdown().await;

    let (tx, rx) = tokio::sync::mpsc::channel::<TrafficFlushBarrier>(1);
    *tracker.writer_tx.write().await = Some(tx.clone());
    used(&tracker, sample(100, 0, 10)).await;
    assert!(tracker.dirty.lock().contains("node-a"));
    let (completed, queued_ack) = tokio::sync::oneshot::channel();
    tx.try_send(TrafficFlushBarrier { completed })
        .expect("barrier should fill the queue");
    drop(tx);

    let remover = tracker.clone();
    let removal =
        tokio::spawn(async move { remover.remove_nodes(vec!["node-a".to_string()]).await });
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(!removal.is_finished());
    let state = tokio::time::timeout(Duration::from_secs(1), tracker.state.lock())
        .await
        .expect("a removal waiting on the queue must not hold the state lock");
    drop(state);

    let writer = tokio::spawn(run_traffic_writer(rx, tracker.clone()));
    tokio::time::timeout(Duration::from_secs(10), removal)
        .await
        .expect("removal should finish")
        .expect("removal task should join")
        .expect("removal should persist");
    queued_ack
        .await
        .expect("queued barrier should be answered")
        .expect("queued barrier should succeed");
    assert_eq!(persisted_usage(&db_path), None);

    *tracker.writer_tx.write().await = None;
    writer.await.expect("writer should stop");
    remove_db(&db_path);
}

#[tokio::test]
async fn writer_persists_only_the_latest_state_of_a_burst() {
    let db_path = test_db_path("burst");
    let tracker = started_tracker(&db_path).await;
    for step in 0..5_000u64 {
        used(&tracker, sample(step, 0, step)).await;
    }
    assert!(tracker.dirty.lock().len() <= 1);
    tokio::time::timeout(Duration::from_secs(10), async {
        while persisted_usage(&db_path) != Some(4_999) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("latest usage should be persisted");
    tracker.shutdown().await;
    assert_eq!(persisted_usage(&db_path), Some(4_999));
    remove_db(&db_path);
}

#[tokio::test]
async fn quota_disable_is_not_undone_by_a_retained_upsert() {
    let db_path = test_db_path("disable-retained");
    let tracker = started_tracker(&db_path).await;
    used(&tracker, sample(100, 0, 10)).await;
    wait_for_health(&tracker, |health| health.last_success_at > 0).await;

    let lock = lock_database(&db_path);
    used(&tracker, sample(400, 0, 20)).await;
    wait_for_health(&tracker, |health| health.write_degraded).await;
    let disabled = tracker
        .record("node-a", sample(500, 0, 30), BIDI, false)
        .await
        .expect("ledger should be available");
    assert!(disabled.is_none());
    drop(lock);
    tokio::time::timeout(Duration::from_secs(10), async {
        while persisted_usage(&db_path).is_some() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("disable should remove the row");
    tracker.shutdown().await;

    let restarted = started_tracker(&db_path).await;
    assert!(restarted.usages().await.is_empty());
    restarted.shutdown().await;
    remove_db(&db_path);
}

#[tokio::test]
async fn dirty_marker_of_a_removed_node_does_not_recreate_its_ledger() {
    let db_path = test_db_path("dropped-removed");
    let tracker = started_tracker(&db_path).await;
    used(&tracker, sample(100, 0, 10)).await;
    wait_for_health(&tracker, |health| health.last_success_at > 0).await;
    tracker.state.lock().await.remove("node-a");
    tracker.dirty.lock().insert("node-a".to_string());
    tokio::time::timeout(Duration::from_secs(10), async {
        while persisted_usage(&db_path).is_some() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("stale marker should resolve to a delete");
    tracker.shutdown().await;
    remove_db(&db_path);
}

#[tokio::test]
async fn shutdown_reports_writes_that_could_not_be_persisted() {
    let db_path = test_db_path("shutdown-unpersisted");
    let tracker = started_tracker(&db_path).await;
    used(&tracker, sample(100, 0, 10)).await;
    wait_for_health(&tracker, |health| health.last_success_at > 0).await;

    let lock = lock_database(&db_path);
    used(&tracker, sample(400, 0, 20)).await;
    tracker.shutdown().await;
    let health = tracker.health();
    assert!(!health.available);
    assert_eq!(health.pending_writes, 1);
    drop(lock);

    // 账本回到最后一次提交的值，但基线也一起回退，所以下一帧能把差额补回。
    let restarted = started_tracker(&db_path).await;
    assert_eq!(restarted.usages().await[0].used_bytes, 0);
    assert_eq!(used(&restarted, sample(450, 0, 30)).await, 350);
    restarted.shutdown().await;
    remove_db(&db_path);
}

fn seed_ledger_row(db_path: &Path, accounting: &str, used_bytes: &str) {
    let connection = rusqlite::Connection::open(db_path).expect("db should open");
    connection
        .execute_batch(
            "CREATE TABLE IF NOT EXISTS traffic_usage (node_id TEXT PRIMARY KEY NOT NULL, cycle_started_at INTEGER NOT NULL, accounting TEXT NOT NULL, used_bytes INTEGER NOT NULL, last_rx_bytes INTEGER NOT NULL, last_tx_bytes INTEGER NOT NULL);",
        )
        .expect("schema should apply");
    connection
        .execute(
            "INSERT INTO traffic_usage VALUES ('node-a', ?1, ?2, ?3, 0, 0)",
            rusqlite::params![
                utc_month_start(Utc::now()).timestamp(),
                accounting,
                used_bytes
            ],
        )
        .expect("row should insert");
}

#[tokio::test]
async fn invalid_ledger_rows_make_the_ledger_unavailable() {
    for (accounting, used_bytes) in [
        ("bidirectional", "not-a-number"),
        ("bidirectional", "-1"),
        ("sideways", "1"),
    ] {
        let db_path = test_db_path("invalid-row");
        seed_ledger_row(&db_path, accounting, used_bytes);
        let tracker = TrafficTracker::new(Arc::new(db_path.clone()), 0);
        assert!(
            tracker.initialize().await.is_err(),
            "{accounting}/{used_bytes}"
        );
        assert!(!tracker.health().available);
        remove_db(&db_path);
    }
}

#[tokio::test]
async fn ledger_load_is_retried_and_preserves_repaired_usage() {
    let db_path = test_db_path("load-retry");
    seed_ledger_row(&db_path, "bidirectional", "not-a-number");
    let tracker = TrafficTracker::new(Arc::new(db_path.clone()), 0);
    tracker.start(Duration::from_millis(20)).await;
    assert!(!tracker.health().available);
    assert!(!tracker.take_recovered_after_startup());
    assert!(
        tracker
            .record("node-a", sample(1, 1, 1), BIDI, true)
            .await
            .is_err()
    );

    rusqlite::Connection::open(&db_path)
        .expect("db should open")
        .execute("UPDATE traffic_usage SET used_bytes = 700", [])
        .expect("row should be repaired");
    wait_for_health(&tracker, |health| health.available).await;
    assert!(tracker.take_recovered_after_startup());
    assert!(!tracker.take_recovered_after_startup());
    assert_eq!(tracker.usages().await[0].used_bytes, 700);

    tracker.shutdown().await;
    remove_db(&db_path);
}

#[tokio::test]
async fn shutdown_stops_a_pending_ledger_load_retry() {
    let db_path = test_db_path("load-retry-stop");
    seed_ledger_row(&db_path, "sideways", "1");
    let tracker = TrafficTracker::new(Arc::new(db_path.clone()), 0);
    tracker.start(Duration::from_millis(5)).await;
    tracker.shutdown().await;
    rusqlite::Connection::open(&db_path)
        .expect("db should open")
        .execute("UPDATE traffic_usage SET accounting = 'inbound'", [])
        .expect("row should be repaired");
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(!tracker.health().available);
    remove_db(&db_path);
}
