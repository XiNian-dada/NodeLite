//! 历史数据库初始化与权限加固。

use std::ffi::OsString;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result};
use rusqlite::{Connection, OpenFlags};

use crate::fs_security::{create_private_dir_all, ensure_directory_mode};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

/// 建库:如果父目录不存在则创建,然后建表 / 建索引并收紧权限。
/// 返回已配置好的持久化连接(WAL 模式 + busy_timeout),供后续写入/查询复用。
pub(super) fn initialize_database(
    db_path: &PathBuf,
    sqlite_busy_timeout_secs: u64,
) -> Result<Connection> {
    if let Some(parent) = db_path.parent()
        && !parent.as_os_str().is_empty()
    {
        create_private_dir_all(parent)?;
    }

    let connection = open_database_connection(db_path, true, sqlite_busy_timeout_secs)?;
    connection.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS history_points (
            node_id TEXT NOT NULL,
            recorded_at INTEGER NOT NULL,
            cpu_usage_percent REAL,
            load_one REAL,
            load_five REAL,
            load_fifteen REAL,
            memory_used_percent REAL NOT NULL,
            rx_bytes_per_sec REAL,
            tx_bytes_per_sec REAL,
            latency_ms INTEGER,
            packet_loss_percent REAL,
            disk_used_percent REAL
        );
        "#,
    )?;
    migrate_nullable_cpu_usage(&connection)?;
    migrate_history_metric_columns(&connection)?;
    ensure_history_indexes(&connection)?;
    harden_database_artifacts(db_path)?;

    Ok(connection)
}

fn migrate_nullable_cpu_usage(connection: &Connection) -> Result<()> {
    let cpu_not_null = connection
        .prepare("PRAGMA table_info(history_points)")?
        .query_map([], |row| {
            let column_name: String = row.get(1)?;
            let not_null: i64 = row.get(3)?;
            Ok((column_name, not_null))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?
        .into_iter()
        .any(|(column_name, not_null)| column_name == "cpu_usage_percent" && not_null != 0);
    if !cpu_not_null {
        return Ok(());
    }

    connection.execute_batch(
        r#"
        DROP INDEX IF EXISTS idx_history_points_node_time;
        DROP INDEX IF EXISTS idx_history_points_covering_metrics;
        ALTER TABLE history_points RENAME TO history_points_legacy_not_null_cpu;
        CREATE TABLE history_points (
            node_id TEXT NOT NULL,
            recorded_at INTEGER NOT NULL,
            cpu_usage_percent REAL,
            load_one REAL,
            load_five REAL,
            load_fifteen REAL,
            memory_used_percent REAL NOT NULL,
            rx_bytes_per_sec REAL,
            tx_bytes_per_sec REAL,
            latency_ms INTEGER,
            packet_loss_percent REAL,
            disk_used_percent REAL
        );
        INSERT INTO history_points (
            node_id,
            recorded_at,
            cpu_usage_percent,
            load_one,
            load_five,
            load_fifteen,
            memory_used_percent,
            rx_bytes_per_sec,
            tx_bytes_per_sec,
            latency_ms,
            packet_loss_percent,
            disk_used_percent
        )
        SELECT
            node_id,
            recorded_at,
            cpu_usage_percent,
            NULL,
            NULL,
            NULL,
            memory_used_percent,
            rx_bytes_per_sec,
            tx_bytes_per_sec,
            latency_ms,
            NULL,
            disk_used_percent
        FROM history_points_legacy_not_null_cpu;
        DROP TABLE history_points_legacy_not_null_cpu;
        "#,
    )?;
    Ok(())
}

fn migrate_history_metric_columns(connection: &Connection) -> Result<()> {
    let existing_columns = connection
        .prepare("PRAGMA table_info(history_points)")?
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for (column, sql) in [
        (
            "load_one",
            "ALTER TABLE history_points ADD COLUMN load_one REAL",
        ),
        (
            "load_five",
            "ALTER TABLE history_points ADD COLUMN load_five REAL",
        ),
        (
            "load_fifteen",
            "ALTER TABLE history_points ADD COLUMN load_fifteen REAL",
        ),
        (
            "packet_loss_percent",
            "ALTER TABLE history_points ADD COLUMN packet_loss_percent REAL",
        ),
    ] {
        if !existing_columns.iter().any(|existing| existing == column) {
            connection.execute(sql, [])?;
        }
    }
    Ok(())
}

fn ensure_history_indexes(connection: &Connection) -> Result<()> {
    connection.execute(
        "CREATE INDEX IF NOT EXISTS idx_history_points_node_time ON history_points (node_id, recorded_at)",
        [],
    )?;

    let expected_covering_columns = [
        "node_id",
        "recorded_at",
        "cpu_usage_percent",
        "load_one",
        "load_five",
        "load_fifteen",
        "memory_used_percent",
        "rx_bytes_per_sec",
        "tx_bytes_per_sec",
        "latency_ms",
        "packet_loss_percent",
        "disk_used_percent",
    ];
    let existing_covering_columns = covering_index_columns(connection)?;
    if existing_covering_columns
        .as_deref()
        .is_some_and(|columns| columns == expected_covering_columns)
    {
        return Ok(());
    }

    connection.execute(
        "DROP INDEX IF EXISTS idx_history_points_covering_metrics",
        [],
    )?;
    connection.execute_batch(
        r#"
        CREATE INDEX IF NOT EXISTS idx_history_points_node_time
            ON history_points (node_id, recorded_at);
        CREATE INDEX IF NOT EXISTS idx_history_points_covering_metrics
            ON history_points (
                node_id,
                recorded_at,
                cpu_usage_percent,
                load_one,
                load_five,
                load_fifteen,
                memory_used_percent,
                rx_bytes_per_sec,
                tx_bytes_per_sec,
                latency_ms,
                packet_loss_percent,
                disk_used_percent
            );
        "#,
    )?;
    Ok(())
}

fn covering_index_columns(connection: &Connection) -> Result<Option<Vec<String>>> {
    let exists: i64 = connection.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'index' AND name = ?1",
        ["idx_history_points_covering_metrics"],
        |row| row.get(0),
    )?;
    if exists == 0 {
        return Ok(None);
    }
    let mut statement =
        connection.prepare("PRAGMA index_info(idx_history_points_covering_metrics)")?;
    let columns = statement
        .query_map([], |row| row.get::<_, String>(2))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(Some(columns))
}

/// 打开查询专用连接。初始化阶段已经确保库文件和 schema 存在,
/// 这里使用只读连接,避免 history 查询与 writer task 争用同一个连接 mutex。
pub(super) fn open_read_connection(
    db_path: &PathBuf,
    sqlite_busy_timeout_secs: u64,
    history_read_cache_kib: u64,
) -> Result<Connection> {
    let connection = Connection::open_with_flags(db_path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .with_context(|| {
            format!(
                "failed to open read-only history database {}",
                db_path.display()
            )
        })?;
    connection
        .busy_timeout(Duration::from_secs(sqlite_busy_timeout_secs))
        .context("failed to configure sqlite read busy timeout")?;
    let history_read_cache_kib = i64::try_from(history_read_cache_kib)
        .context("history read page cache exceeds SQLite integer range")?;
    connection
        .pragma_update(None, "cache_size", -history_read_cache_kib)
        .context("failed to configure sqlite read page cache")?;
    Ok(connection)
}

/// 打开 SQLite 连接,可选启用 WAL 模式以提升并发写入吞吐。
fn open_database_connection(
    db_path: &PathBuf,
    enable_wal: bool,
    sqlite_busy_timeout_secs: u64,
) -> Result<Connection> {
    let connection = Connection::open(db_path)
        .with_context(|| format!("failed to open history database {}", db_path.display()))?;
    connection
        .busy_timeout(Duration::from_secs(sqlite_busy_timeout_secs))
        .context("failed to configure sqlite busy timeout")?;
    if enable_wal {
        connection
            .pragma_update(None, "journal_mode", "WAL")
            .context("failed to enable sqlite WAL mode")?;
    }
    Ok(connection)
}

/// 收紧主库文件以及 WAL / SHM 辅助文件的权限。
pub(super) fn harden_database_artifacts(db_path: &PathBuf) -> Result<()> {
    if let Some(parent) = db_path.parent()
        && !parent.as_os_str().is_empty()
    {
        ensure_directory_mode(parent, 0o700)?;
    }
    harden_path_permissions(db_path)?;
    for suffix in ["-wal", "-shm"] {
        let mut artifact = OsString::from(db_path.as_os_str());
        artifact.push(suffix);
        let artifact = PathBuf::from(artifact);
        if artifact.exists() {
            harden_path_permissions(&artifact)?;
        }
    }
    Ok(())
}

fn harden_path_permissions(path: &PathBuf) -> Result<()> {
    #[cfg(unix)]
    {
        if path.exists() {
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
                .with_context(|| format!("failed to chmod {}", path.display()))?;
        }
    }

    #[cfg(not(unix))]
    {
        let _ = path;
    }

    Ok(())
}
