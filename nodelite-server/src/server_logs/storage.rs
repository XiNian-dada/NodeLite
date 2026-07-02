//! Server 日志持久化存储。

use std::path::Path;

use anyhow::{Context, Result};
use rusqlite::Connection;

/// Server 日志表 schema。
const SERVER_LOGS_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS server_logs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    occurred_at TEXT NOT NULL,
    level TEXT NOT NULL,
    target TEXT NOT NULL,
    message TEXT NOT NULL,
    sequence INTEGER NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_server_logs_occurred
    ON server_logs(occurred_at DESC);
CREATE INDEX IF NOT EXISTS idx_server_logs_level
    ON server_logs(level, occurred_at DESC);
CREATE INDEX IF NOT EXISTS idx_server_logs_created
    ON server_logs(created_at DESC);
"#;

/// 初始化 server logs 数据库:创建表和索引。
pub(super) fn initialize_database(db_path: &Path) -> Result<Connection> {
    let parent = db_path
        .parent()
        .context("server logs db path must have parent directory")?;
    std::fs::create_dir_all(parent)
        .with_context(|| format!("create server logs db parent dir {}", parent.display()))?;

    let conn = Connection::open(db_path)
        .with_context(|| format!("open server logs db at {}", db_path.display()))?;

    harden_file_permissions(db_path)?;

    conn.pragma_update(None, "journal_mode", "WAL")
        .context("set server logs db to WAL mode")?;
    conn.pragma_update(None, "synchronous", "NORMAL")
        .context("set server logs db synchronous to NORMAL")?;
    conn.pragma_update(None, "foreign_keys", "ON")
        .context("enable foreign keys in server logs db")?;

    conn.execute_batch(SERVER_LOGS_SCHEMA)
        .context("create server logs schema")?;

    Ok(conn)
}

/// 打开只读连接用于查询。
pub(super) fn open_read_connection(db_path: &Path) -> Result<Connection> {
    let conn = Connection::open_with_flags(
        db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .with_context(|| format!("open server logs db read-only at {}", db_path.display()))?;
    Ok(conn)
}

fn harden_file_permissions(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
            .with_context(|| format!("chmod 0600 {}", path.display()))?;
    }

    #[cfg(not(unix))]
    {
        let _ = path;
    }

    Ok(())
}
