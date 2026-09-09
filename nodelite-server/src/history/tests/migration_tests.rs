//! Interrupted migrations must either recover every sample or preserve both tables.

use super::*;

fn seed_legacy_database(path: &PathBuf) -> rusqlite::Connection {
    let connection = rusqlite::Connection::open(path).expect("open legacy database");
    connection
        .execute_batch(
            "CREATE TABLE history_points (
                node_id TEXT NOT NULL, recorded_at INTEGER NOT NULL,
                cpu_usage_percent REAL NOT NULL, memory_used_percent REAL,
                rx_bytes_per_sec REAL, tx_bytes_per_sec REAL,
                latency_ms INTEGER, disk_used_percent REAL
            );
            CREATE INDEX idx_history_points_node_time ON history_points(node_id, recorded_at);
            INSERT INTO history_points VALUES ('legacy-node', 123, 42, 50, 1, 2, 3, 4);",
        )
        .expect("seed old schema and sample");
    connection
}

fn sample_count(connection: &rusqlite::Connection) -> i64 {
    connection
        .query_row("SELECT COUNT(*) FROM history_points", [], |row| row.get(0))
        .expect("count active samples")
}

#[test]
fn history_migration_preserves_samples_and_is_idempotent() {
    let path = temp_history_db_path("migration-preserves-samples");
    drop(seed_legacy_database(&path));
    for _ in 0..2 {
        let connection = initialize_database(&path, 5).expect("migrate and reopen");
        assert_eq!(sample_count(&connection), 1);
        let cpu: f64 = connection
            .query_row("SELECT cpu_usage_percent FROM history_points", [], |row| {
                row.get(0)
            })
            .expect("read migrated sample");
        assert_eq!(cpu, 42.0);
    }
    std::fs::remove_dir_all(path.parent().expect("test directory")).expect("clean test database");
}

#[test]
fn history_migration_rolls_back_ddl_and_indexes_when_copy_fails() {
    let path = temp_history_db_path("migration-rollback");
    let legacy = seed_legacy_database(&path);
    // A value rejected by the new schema injects failure after rename and CREATE.
    legacy
        .execute("UPDATE history_points SET memory_used_percent = NULL", [])
        .expect("inject invalid sample");
    drop(legacy);
    assert!(initialize_database(&path, 5).is_err());

    let connection = rusqlite::Connection::open(&path).expect("reopen after failed migration");
    assert_eq!(sample_count(&connection), 1);
    let not_null: bool = connection.query_row(
        "SELECT [notnull] FROM pragma_table_info('history_points') WHERE name = 'cpu_usage_percent'",
        [], |row| row.get(0),
    ).expect("old schema survives");
    assert!(not_null);
    let index_exists: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name = 'idx_history_points_node_time')",
        [], |row| row.get(0),
    ).expect("old index survives");
    assert!(index_exists);
    connection
        .execute("UPDATE history_points SET memory_used_percent = 50", [])
        .expect("repair injected fault");
    drop(connection);
    let connection = initialize_database(&path, 5).expect("retry migration");
    assert_eq!(sample_count(&connection), 1);
    drop(connection);
    std::fs::remove_dir_all(path.parent().expect("test directory")).expect("clean test database");
}

#[test]
fn history_recovers_legacy_table_left_before_or_after_empty_table_creation() {
    for active_table_exists in [false, true] {
        let path = temp_history_db_path("migration-recovery");
        let connection = seed_legacy_database(&path);
        connection
            .execute(
                "ALTER TABLE history_points RENAME TO history_points_legacy_not_null_cpu",
                [],
            )
            .expect("simulate interrupted old migration");
        if active_table_exists {
            connection.execute("CREATE TABLE history_points AS SELECT * FROM history_points_legacy_not_null_cpu WHERE 0", []).expect("simulate empty replacement table");
        }
        drop(connection);
        let connection = initialize_database(&path, 5).expect("recover interrupted migration");
        assert_eq!(sample_count(&connection), 1);
        let legacy_exists: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name = 'history_points_legacy_not_null_cpu')",
            [], |row| row.get(0),
        ).expect("recovery completed");
        assert!(!legacy_exists);
        drop(connection);
        std::fs::remove_dir_all(path.parent().expect("test directory"))
            .expect("clean test database");
    }
}

#[test]
fn history_refuses_ambiguous_legacy_recovery_without_changing_either_table() {
    let path = temp_history_db_path("migration-ambiguous");
    let connection = seed_legacy_database(&path);
    connection
        .execute_batch(
            "ALTER TABLE history_points RENAME TO history_points_legacy_not_null_cpu;
        CREATE TABLE history_points AS SELECT * FROM history_points_legacy_not_null_cpu;",
        )
        .expect("simulate interruption after copying samples");
    drop(connection);
    let error = initialize_database(&path, 5).expect_err("ambiguous recovery must be explicit");
    assert!(error.to_string().contains("recover the legacy table"));
    let connection = rusqlite::Connection::open(&path).expect("inspect preserved database");
    assert_eq!(sample_count(&connection), 1);
    let legacy_count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM history_points_legacy_not_null_cpu",
            [],
            |row| row.get(0),
        )
        .expect("read legacy samples");
    assert_eq!(legacy_count, 1);
    drop(connection);
    std::fs::remove_dir_all(path.parent().expect("test directory")).expect("clean test database");
}
