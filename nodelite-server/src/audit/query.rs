//! Audit log query helpers.

use anyhow::anyhow;
use chrono::{TimeZone, Utc};
use rusqlite::{Connection, params};
use serde_json::Value;

use super::{AuditEvent, AuditEventType, AuditLogError, AuditQuery};

const AUDIT_QUERY_SQL: &str = r#"
SELECT id, timestamp, event_type, user, node_id, ip_address, user_agent, success, details
FROM audit_log
WHERE (?1 IS NULL OR timestamp >= ?1)
  AND (?2 IS NULL OR timestamp <= ?2)
  AND (?3 IS NULL OR event_type = ?3)
  AND (?4 IS NULL OR success = ?4)
ORDER BY timestamp DESC, id DESC
LIMIT ?5
"#;

pub(super) fn query_events(
    connection: &Connection,
    query: &AuditQuery,
) -> std::result::Result<Vec<AuditEvent>, AuditLogError> {
    let start = query.start.map(|value| value.timestamp());
    let end = query.end.map(|value| value.timestamp());
    let event_type = query.event_type.map(AuditEventType::as_str);
    let success = query.success.map(|value| value as i64);

    let mut statement = connection
        .prepare(AUDIT_QUERY_SQL)
        .map_err(|error| AuditLogError::Query(anyhow!("failed to prepare audit query: {error}")))?;
    let rows = statement
        .query_map(
            params![start, end, event_type, success, query.limit as i64],
            |row| {
                let event_type = row.get::<_, String>(2)?;
                let details = row.get::<_, String>(8)?;
                let timestamp = row.get::<_, i64>(1)?;
                let event_type = AuditEventType::parse(&event_type).ok_or_else(|| {
                    rusqlite::Error::FromSqlConversionFailure(
                        2,
                        rusqlite::types::Type::Text,
                        Box::new(std::io::Error::other(format!(
                            "unknown audit event type {event_type}"
                        ))),
                    )
                })?;
                let details = serde_json::from_str::<Value>(&details).map_err(|error| {
                    rusqlite::Error::FromSqlConversionFailure(
                        7,
                        rusqlite::types::Type::Text,
                        Box::new(error),
                    )
                })?;
                Ok(AuditEvent {
                    id: row.get(0)?,
                    timestamp: Utc.timestamp_opt(timestamp, 0).single().ok_or_else(|| {
                        rusqlite::Error::FromSqlConversionFailure(
                            1,
                            rusqlite::types::Type::Integer,
                            Box::new(std::io::Error::other(format!(
                                "invalid audit timestamp {timestamp}"
                            ))),
                        )
                    })?,
                    event_type,
                    user: row.get(3)?,
                    node_id: row.get(4)?,
                    ip_address: row.get(5)?,
                    user_agent: row.get(6)?,
                    success: row.get::<_, i64>(7)? != 0,
                    details,
                })
            },
        )
        .map_err(|error| AuditLogError::Query(anyhow!("failed to execute audit query: {error}")))?;

    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|error| AuditLogError::Query(anyhow!("failed to decode audit rows: {error}")))
}
