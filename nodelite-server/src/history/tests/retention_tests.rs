//! 保留清理的索引访问路径与分批删除测试。

use chrono::TimeZone;

use super::*;
use crate::history::{HISTORY_PRUNE_BATCH_ROWS, prune_history_before};

fn seed_rows(connection: &mut rusqlite::Connection, start: i64, count: i64) {
    let tx = connection.transaction().expect("seed transaction");
    {
        let mut insert = tx
            .prepare(
                "INSERT INTO history_points (node_id, recorded_at, memory_used_percent)
                 VALUES (?1, ?2, 50.0)",
            )
            .expect("seed insert should prepare");
        for index in 0..count {
            insert
                .execute(rusqlite::params![
                    format!("node-{}", index % 200),
                    start + index
                ])
                .expect("seed row should insert");
        }
    }
    tx.commit().expect("seed commit");
}

fn query_plan(
    connection: &rusqlite::Connection,
    sql: &str,
    params: &[&dyn rusqlite::ToSql],
) -> String {
    let mut statement = connection
        .prepare(&format!("EXPLAIN QUERY PLAN {sql}"))
        .expect("plan should prepare");
    statement
        .query_map(params, |row| row.get::<_, String>(3))
        .expect("plan should run")
        .collect::<Result<Vec<_>, _>>()
        .expect("plan rows should decode")
        .join("\n")
}

#[test]
fn retention_delete_plan_uses_recorded_at_index() {
    let db_path = temp_history_db_path("retention-plan");
    let mut connection = initialize_database(&db_path, 5).expect("database should initialize");
    seed_rows(&mut connection, 1_800_000_000, 50_000);
    connection.execute_batch("ANALYZE").expect("analyze");

    let plain = query_plan(
        &connection,
        "DELETE FROM history_points WHERE recorded_at < ?1",
        &[&1_800_000_100_i64],
    );
    let batched = query_plan(
        &connection,
        "SELECT rowid FROM history_points INDEXED BY idx_history_points_recorded_at
         WHERE recorded_at < ?1 LIMIT ?2",
        &[&1_800_000_100_i64, &HISTORY_PRUNE_BATCH_ROWS],
    );
    for plan in [plain, batched] {
        assert!(
            plan.contains("idx_history_points_recorded_at"),
            "retention cleanup should use the time index, got:\n{plan}"
        );
    }
    let _ = std::fs::remove_dir_all(db_path.parent().expect("temp dir"));
}

#[test]
fn retention_index_is_added_to_existing_database() {
    let db_path = temp_history_db_path("retention-migrate");
    drop(initialize_database(&db_path, 5).expect("database should initialize"));
    let raw = rusqlite::Connection::open(&db_path).expect("raw open");
    raw.execute("DROP INDEX idx_history_points_recorded_at", [])
        .expect("drop index to simulate legacy db");
    drop(raw);

    let connection = initialize_database(&db_path, 5).expect("database should re-initialize");
    let exists: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'index' AND name = ?1",
            ["idx_history_points_recorded_at"],
            |row| row.get(0),
        )
        .expect("index lookup");
    assert_eq!(exists, 1);
    let _ = std::fs::remove_dir_all(db_path.parent().expect("temp dir"));
}

#[test]
fn prune_deletes_in_bounded_batches_and_keeps_recent_rows() {
    let db_path = temp_history_db_path("retention-batches");
    let mut connection = initialize_database(&db_path, 5).expect("database should initialize");
    let expired = HISTORY_PRUNE_BATCH_ROWS * 2 + 17;
    let start = 1_800_000_000_i64;
    seed_rows(&mut connection, start, expired + 100);

    let cutoff = Utc
        .timestamp_opt(start + expired, 0)
        .single()
        .expect("valid cutoff");
    let deleted = prune_history_before(&connection, cutoff).expect("prune should succeed");
    assert_eq!(deleted as i64, expired);
    let (remaining, oldest): (i64, i64) = connection
        .query_row(
            "SELECT COUNT(*), MIN(recorded_at) FROM history_points",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("count");
    assert_eq!(remaining, 100);
    assert_eq!(oldest, start + expired);
    let _ = std::fs::remove_dir_all(db_path.parent().expect("temp dir"));
}

#[test]
fn write_batch_with_prune_commits_samples_and_removes_expired_rows() {
    let db_path = temp_history_db_path("retention-write");
    let mut connection = initialize_database(&db_path, 5).expect("database should initialize");
    let hardened = AtomicBool::new(false);
    let now = Utc::now();
    let old = HistoryPoint {
        node_id: "hk-01".to_string(),
        recorded_at: now - Duration::days(30),
        cpu_usage_percent: Some(1.0),
        load_one: None,
        load_five: None,
        load_fifteen: None,
        memory_used_percent: 50.0,
        rx_bytes_per_sec: None,
        tx_bytes_per_sec: None,
        latency_ms: None,
        packet_loss_percent: None,
        disk_used_percent: None,
    };
    write_history_point(&db_path, &mut connection, &old, None, &hardened).expect("old row");
    let fresh = HistoryPoint {
        recorded_at: now,
        ..old.clone()
    };
    write_history_point(
        &db_path,
        &mut connection,
        &fresh,
        Some(now - Duration::days(14)),
        &hardened,
    )
    .expect("fresh row with prune");
    let timestamps: Vec<i64> = connection
        .prepare("SELECT recorded_at FROM history_points")
        .expect("prepare")
        .query_map([], |row| row.get(0))
        .expect("query")
        .collect::<Result<_, _>>()
        .expect("decode");
    assert_eq!(timestamps, vec![now.timestamp()]);
    let _ = std::fs::remove_dir_all(db_path.parent().expect("temp dir"));
}
