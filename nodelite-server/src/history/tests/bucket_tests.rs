//! 历史查询分桶边界测试：桶数不超过预算，且请求结束点的采样不会被截掉。

use chrono::TimeZone;

use super::*;

const START: i64 = 1_800_000_000;

fn at(ts: i64) -> DateTime<Utc> {
    Utc.timestamp_opt(ts, 0).single().expect("valid timestamp")
}

fn seeded(test_name: &str, offsets: &[i64], spike_offset: i64) -> rusqlite::Connection {
    let db_path = temp_history_db_path(test_name);
    let mut connection = initialize_database(&db_path, 5).expect("database should initialize");
    let tx = connection.transaction().expect("seed transaction");
    for &offset in offsets {
        let cpu = if offset == spike_offset { 99.0 } else { 1.0 };
        tx.execute(
            "INSERT INTO history_points (node_id, recorded_at, cpu_usage_percent, memory_used_percent)
             VALUES ('hk-01', ?1, ?2, 50.0)",
            rusqlite::params![START + offset, cpu],
        )
        .expect("seed row");
    }
    tx.commit().expect("seed commit");
    connection
}

#[test]
fn exact_multiple_span_keeps_end_sample_within_budget() {
    let offsets: Vec<i64> = (0..=60).map(|minute| minute * 60).collect();
    let connection = seeded("bucket-exact", &offsets, 3600);
    let points = query_history_between(&connection, "hk-01", at(START), at(START + 3600), 60)
        .expect("query should succeed");
    assert!(points.len() <= 60, "got {} buckets", points.len());
    let last = points.last().expect("at least one bucket");
    assert_eq!(last.recorded_at.timestamp(), START + 3600);
    assert!(
        points
            .iter()
            .any(|point| point.cpu_usage_percent.is_some_and(|cpu| cpu > 1.0)),
        "end-point spike must survive bucketing"
    );
}

#[test]
fn non_multiple_span_keeps_end_sample_within_budget() {
    let offsets: Vec<i64> = (0..=37).map(|step| step * 97).collect();
    let end = 37 * 97;
    let connection = seeded("bucket-uneven", &offsets, end);
    let points = query_history_between(&connection, "hk-01", at(START), at(START + end), 7)
        .expect("query should succeed");
    assert!(points.len() <= 7);
    assert_eq!(
        points.last().map(|point| point.recorded_at.timestamp()),
        Some(START + end)
    );
}

#[test]
fn one_sample_per_second_fills_budget_exactly() {
    let offsets: Vec<i64> = (0..=59).collect();
    let connection = seeded("bucket-budget", &offsets, 59);
    let points = query_history_between(&connection, "hk-01", at(START), at(START + 59), 60)
        .expect("query should succeed");
    assert_eq!(points.len(), 60);
    assert_eq!(points[59].recorded_at.timestamp(), START + 59);
    assert_eq!(points[59].cpu_usage_percent, Some(99.0));
}

#[test]
fn empty_end_and_single_point_ranges() {
    let connection = seeded("bucket-sparse", &[0, 1800], -1);
    let points = query_history_between(&connection, "hk-01", at(START), at(START + 3600), 60)
        .expect("query should succeed");
    assert_eq!(points.len(), 2);
    assert_eq!(points[1].recorded_at.timestamp(), START + 1800);

    let single = query_history_between(&connection, "hk-01", at(START), at(START), 60)
        .expect("single-instant query should succeed");
    assert_eq!(single.len(), 1);
    assert_eq!(single[0].recorded_at.timestamp(), START);
}
