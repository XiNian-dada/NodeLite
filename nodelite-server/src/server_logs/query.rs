//! Server 日志查询。

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use rusqlite::Connection;

/// 查询时间范围内的日志。
pub(super) fn query_logs_by_time_range(
    conn: &Connection,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    level: Option<&str>,
    limit: usize,
) -> Result<Vec<ServerLogEntryWithLevel>> {
    let start_str = start.to_rfc3339();
    let end_str = end.to_rfc3339();

    let mut stmt = if level.is_some() {
        conn.prepare_cached(
            "SELECT occurred_at, level, target, message
             FROM server_logs
             WHERE occurred_at >= ?1 AND occurred_at <= ?2 AND level = ?3
             ORDER BY occurred_at DESC
             LIMIT ?4",
        )?
    } else {
        conn.prepare_cached(
            "SELECT occurred_at, level, target, message
             FROM server_logs
             WHERE occurred_at >= ?1 AND occurred_at <= ?2
             ORDER BY occurred_at DESC
             LIMIT ?3",
        )?
    };

    let rows: Vec<ServerLogEntryWithLevel> = if let Some(filter_level) = level {
        stmt.query_map([&start_str, &end_str, filter_level, &limit.to_string()], |row| {
            Ok(ServerLogEntryWithLevel {
                occurred_at: row.get(0)?,
                level: row.get(1)?,
                target: row.get(2)?,
                message: row.get(3)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?
    } else {
        stmt.query_map([&start_str, &end_str, &limit.to_string()], |row| {
            Ok(ServerLogEntryWithLevel {
                occurred_at: row.get(0)?,
                level: row.get(1)?,
                target: row.get(2)?,
                message: row.get(3)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?
    };

    let mut entries = rows;
    entries.reverse();
    Ok(entries)
}

/// 获取数据库文件大小(字节)。
pub(super) fn database_size_bytes(conn: &Connection) -> Result<u64> {
    let page_count: i64 = conn
        .query_row("PRAGMA page_count", [], |row| row.get(0))
        .context("query page_count")?;
    let page_size: i64 = conn
        .query_row("PRAGMA page_size", [], |row| row.get(0))
        .context("query page_size")?;
    Ok((page_count * page_size) as u64)
}

/// 删除最旧的日志直到大小低于限制。
pub(super) fn prune_by_size(conn: &mut Connection, max_size_bytes: u64) -> Result<usize> {
    let current_size = database_size_bytes(conn)?;
    if current_size <= max_size_bytes {
        return Ok(0);
    }

    let tx = conn.transaction().context("begin prune transaction")?;
    let deleted: usize = tx
        .execute(
            "DELETE FROM server_logs
             WHERE id IN (
                 SELECT id FROM server_logs
                 ORDER BY created_at ASC
                 LIMIT (SELECT COUNT(*) / 4 FROM server_logs)
             )",
            [],
        )
        .context("delete oldest server logs")?;
    tx.commit().context("commit prune transaction")?;

    // VACUUM 回收空间
    conn.execute("VACUUM", [])
        .context("vacuum after server log prune")?;

    Ok(deleted)
}

#[derive(Debug, Clone)]
pub struct ServerLogEntryWithLevel {
    pub occurred_at: String,
    pub level: String,
    pub target: String,
    pub message: String,
}
