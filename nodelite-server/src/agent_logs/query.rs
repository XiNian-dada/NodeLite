//! Agent 日志查询。

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use nodelite_proto::{AgentLogEntry, NoticeLevel};
use rusqlite::Connection;

/// 查询某节点的日志,按时间倒序。
pub(super) fn query_logs_by_node(
    conn: &Connection,
    node_id: &str,
    limit: usize,
) -> Result<Vec<AgentLogEntry>> {
    let mut stmt = conn
        .prepare_cached(
            "SELECT occurred_at, level, message
             FROM agent_logs
             WHERE node_id = ?1
             ORDER BY occurred_at DESC
             LIMIT ?2",
        )
        .context("prepare agent log query by node")?;

    let rows = stmt
        .query_map([node_id, &limit.to_string()], |row| {
            Ok(AgentLogEntry {
                occurred_at: row.get(0)?,
                level: parse_level(&row.get::<_, String>(1)?),
                message: row.get(2)?,
            })
        })
        .context("execute agent log query by node")?;

    let mut entries = Vec::new();
    for row in rows {
        entries.push(row.context("fetch agent log row")?);
    }
    entries.reverse(); // 返回升序
    Ok(entries)
}

/// 查询时间范围内的日志。
pub(super) fn query_logs_by_time_range(
    conn: &Connection,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    node_id: Option<&str>,
    level: Option<&str>,
    limit: usize,
) -> Result<Vec<AgentLogEntryWithNode>> {
    let start_str = start.to_rfc3339();
    let end_str = end.to_rfc3339();

    let mut stmt = if node_id.is_some() {
        conn.prepare_cached(
            "SELECT node_id, occurred_at, level, message
             FROM agent_logs
             WHERE occurred_at >= ?1 AND occurred_at <= ?2 AND node_id = ?3
             ORDER BY occurred_at DESC
             LIMIT ?4",
        )?
    } else {
        conn.prepare_cached(
            "SELECT node_id, occurred_at, level, message
             FROM agent_logs
             WHERE occurred_at >= ?1 AND occurred_at <= ?2
             ORDER BY occurred_at DESC
             LIMIT ?3",
        )?
    };

    let rows: Vec<AgentLogEntryWithNode> = if let Some(nid) = node_id {
        stmt.query_map([&start_str, &end_str, nid, &limit.to_string()], |row| {
            Ok(AgentLogEntryWithNode {
                node_id: row.get(0)?,
                occurred_at: row.get(1)?,
                level: parse_level(&row.get::<_, String>(2)?),
                message: row.get(3)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?
    } else {
        stmt.query_map([&start_str, &end_str, &limit.to_string()], |row| {
            Ok(AgentLogEntryWithNode {
                node_id: row.get(0)?,
                occurred_at: row.get(1)?,
                level: parse_level(&row.get::<_, String>(2)?),
                message: row.get(3)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?
    };

    let mut entries: Vec<_> = rows
        .into_iter()
        .filter(|entry| {
            if let Some(filter_level) = level {
                entry.level.as_str() == filter_level
            } else {
                true
            }
        })
        .collect();

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
            "DELETE FROM agent_logs
             WHERE id IN (
                 SELECT id FROM agent_logs
                 ORDER BY created_at ASC
                 LIMIT (SELECT COUNT(*) / 4 FROM agent_logs)
             )",
            [],
        )
        .context("delete oldest agent logs")?;
    tx.commit().context("commit prune transaction")?;

    // VACUUM 回收空间
    conn.execute("VACUUM", [])
        .context("vacuum after agent log prune")?;

    Ok(deleted)
}

#[derive(Debug, Clone)]
pub struct AgentLogEntryWithNode {
    pub node_id: String,
    pub occurred_at: String,
    pub level: NoticeLevel,
    pub message: String,
}

fn parse_level(s: &str) -> NoticeLevel {
    match s {
        "error" => NoticeLevel::Error,
        "warn" => NoticeLevel::Warn,
        "info" => NoticeLevel::Info,
        _ => NoticeLevel::Info,
    }
}
