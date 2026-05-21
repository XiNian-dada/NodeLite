//! 安全审计日志:
//!
//! - 把认证失败、TOTP 校验、Token 误用和限流封禁等安全事件持久化到独立 SQLite;
//! - 查询接口返回结构化事件,供排障或后续接前端/告警系统;
//! - 所有写入都走 best-effort 路径:审计失败只记日志,不反向拖慢主流程。

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use chrono::{DateTime, Duration as ChronoDuration, TimeZone, Utc};
use nodelite_proto::AuditConfig;
use rusqlite::{Connection, params, params_from_iter};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::Mutex;
use tracing::warn;

use crate::fs_security::{create_private_dir_all, ensure_directory_mode};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

const AUDIT_TABLE_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS audit_log (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    timestamp INTEGER NOT NULL,
    event_type TEXT NOT NULL,
    user TEXT,
    node_id TEXT,
    ip_address TEXT NOT NULL,
    user_agent TEXT,
    success INTEGER NOT NULL,
    details TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_audit_log_timestamp ON audit_log(timestamp);
CREATE INDEX IF NOT EXISTS idx_audit_log_event_type ON audit_log(event_type);
CREATE INDEX IF NOT EXISTS idx_audit_log_ip_address ON audit_log(ip_address);
"#;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AuditEventType {
    LoginFailure,
    TotpVerifySuccess,
    TotpVerifyFailure,
    NodeConnected,
    TokenInvalid,
    RateLimitExceeded,
}

impl AuditEventType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::LoginFailure => "login_failure",
            Self::TotpVerifySuccess => "totp_verify_success",
            Self::TotpVerifyFailure => "totp_verify_failure",
            Self::NodeConnected => "node_connected",
            Self::TokenInvalid => "token_invalid",
            Self::RateLimitExceeded => "rate_limit_exceeded",
        }
    }

    pub fn parse(input: &str) -> Option<Self> {
        match input {
            "login_failure" => Some(Self::LoginFailure),
            "totp_verify_success" => Some(Self::TotpVerifySuccess),
            "totp_verify_failure" => Some(Self::TotpVerifyFailure),
            "node_connected" => Some(Self::NodeConnected),
            "token_invalid" => Some(Self::TokenInvalid),
            "rate_limit_exceeded" => Some(Self::RateLimitExceeded),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AuditEvent {
    pub id: i64,
    pub timestamp: DateTime<Utc>,
    pub event_type: AuditEventType,
    pub user: Option<String>,
    pub node_id: Option<String>,
    pub ip_address: String,
    pub user_agent: Option<String>,
    pub success: bool,
    pub details: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NewAuditEvent {
    pub timestamp: DateTime<Utc>,
    pub event_type: AuditEventType,
    pub user: Option<String>,
    pub node_id: Option<String>,
    pub ip_address: String,
    pub user_agent: Option<String>,
    pub success: bool,
    pub details: Value,
}

impl NewAuditEvent {
    pub fn now(event_type: AuditEventType, ip_address: impl Into<String>, success: bool) -> Self {
        Self {
            timestamp: Utc::now(),
            event_type,
            user: None,
            node_id: None,
            ip_address: ip_address.into(),
            user_agent: None,
            success,
            details: Value::Object(Default::default()),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct AuditQuery {
    pub start: Option<DateTime<Utc>>,
    pub end: Option<DateTime<Utc>>,
    pub event_type: Option<AuditEventType>,
    pub success: Option<bool>,
    pub limit: usize,
}

#[derive(Debug)]
pub enum AuditLogError {
    Disabled,
    Query(anyhow::Error),
}

impl std::fmt::Display for AuditLogError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Disabled => f.write_str("audit log is disabled"),
            Self::Query(_) => f.write_str("audit log query failed"),
        }
    }
}

impl std::error::Error for AuditLogError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Query(error) => Some(error.root_cause()),
            _ => None,
        }
    }
}

#[derive(Clone)]
pub struct AuditLog {
    config: Arc<AuditConfig>,
    sqlite_busy_timeout_secs: u64,
    /// SQLite 本身支持并发读,但这里显式串行化写入/清理,避免多个请求同时打开
    /// 独立连接把低频审计路径放大成锁竞争热点。
    write_gate: Arc<Mutex<()>>,
}

impl AuditLog {
    pub fn new(config: AuditConfig, sqlite_busy_timeout_secs: u64) -> Self {
        Self {
            config: Arc::new(config),
            sqlite_busy_timeout_secs,
            write_gate: Arc::new(Mutex::new(())),
        }
    }

    pub async fn initialize(&self) -> Result<()> {
        if !self.config.enabled {
            return Ok(());
        }

        let config = Arc::clone(&self.config);
        let sqlite_busy_timeout_secs = self.sqlite_busy_timeout_secs;
        tokio::task::spawn_blocking(move || {
            let connection = open_audit_connection(&config.db_path, sqlite_busy_timeout_secs)?;
            prune_expired_records(&connection, config.retention_days)?;
            harden_audit_artifacts(&config.db_path)?;
            Ok::<(), anyhow::Error>(())
        })
        .await
        .context("audit log initialization task failed")??;
        Ok(())
    }

    pub async fn record(&self, event: NewAuditEvent) -> Result<()> {
        if !self.should_record(event.event_type) {
            return Ok(());
        }

        let _guard = self.write_gate.lock().await;
        let config = Arc::clone(&self.config);
        let sqlite_busy_timeout_secs = self.sqlite_busy_timeout_secs;
        tokio::task::spawn_blocking(move || {
            let connection = open_audit_connection(&config.db_path, sqlite_busy_timeout_secs)?;
            prune_expired_records(&connection, config.retention_days)?;
            insert_event(&connection, &event)?;
            harden_audit_artifacts(&config.db_path)?;
            Ok::<(), anyhow::Error>(())
        })
        .await
        .context("audit log record task failed")??;
        Ok(())
    }

    pub async fn record_best_effort(&self, event: NewAuditEvent) {
        let event_type = event.event_type;
        if let Err(error) = self.record(event).await {
            warn!(event_type = event_type.as_str(), error = ?error, "failed to persist audit event");
        }
    }

    pub async fn query(&self, query: AuditQuery) -> Result<Vec<AuditEvent>, AuditLogError> {
        if !self.config.enabled {
            return Err(AuditLogError::Disabled);
        }
        let config = Arc::clone(&self.config);
        let sqlite_busy_timeout_secs = self.sqlite_busy_timeout_secs;
        tokio::task::spawn_blocking(move || {
            let connection = open_audit_connection(&config.db_path, sqlite_busy_timeout_secs)
                .map_err(AuditLogError::Query)?;
            query_events(&connection, &query)
        })
        .await
        .map_err(|error| AuditLogError::Query(anyhow!("audit log query task failed: {error}")))?
    }

    fn should_record(&self, event_type: AuditEventType) -> bool {
        if !self.config.enabled {
            return false;
        }

        match event_type {
            AuditEventType::TotpVerifySuccess | AuditEventType::NodeConnected => {
                self.config.log_successful_auth
            }
            AuditEventType::LoginFailure | AuditEventType::TotpVerifyFailure => {
                self.config.log_failed_auth
            }
            AuditEventType::TokenInvalid => self.config.log_token_events,
            AuditEventType::RateLimitExceeded => self.config.log_rate_limit,
        }
    }
}

fn open_audit_connection(path: &Path, sqlite_busy_timeout_secs: u64) -> Result<Connection> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        create_private_dir_all(parent)?;
    }

    let connection = Connection::open(path)
        .with_context(|| format!("failed to open audit database {}", path.display()))?;
    connection
        .busy_timeout(Duration::from_secs(sqlite_busy_timeout_secs))
        .with_context(|| format!("failed to set audit busy timeout for {}", path.display()))?;
    connection
        .execute_batch(AUDIT_TABLE_SQL)
        .with_context(|| format!("failed to initialize audit schema {}", path.display()))?;
    harden_audit_artifacts(path)?;
    Ok(connection)
}

fn insert_event(connection: &Connection, event: &NewAuditEvent) -> Result<()> {
    let details =
        serde_json::to_string(&event.details).context("failed to serialize audit details")?;
    connection
        .execute(
            "INSERT INTO audit_log
             (timestamp, event_type, user, node_id, ip_address, user_agent, success, details)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                event.timestamp.timestamp(),
                event.event_type.as_str(),
                event.user,
                event.node_id,
                event.ip_address,
                event.user_agent,
                event.success as i64,
                details,
            ],
        )
        .context("failed to insert audit event")?;
    Ok(())
}

fn query_events(
    connection: &Connection,
    query: &AuditQuery,
) -> Result<Vec<AuditEvent>, AuditLogError> {
    let mut sql = String::from(
        "SELECT id, timestamp, event_type, user, node_id, ip_address, user_agent, success, details
         FROM audit_log WHERE 1=1",
    );
    let mut values = Vec::<rusqlite::types::Value>::new();

    if let Some(start) = query.start {
        sql.push_str(" AND timestamp >= ?");
        values.push(rusqlite::types::Value::Integer(start.timestamp()));
    }
    if let Some(end) = query.end {
        sql.push_str(" AND timestamp <= ?");
        values.push(rusqlite::types::Value::Integer(end.timestamp()));
    }
    if let Some(event_type) = query.event_type {
        sql.push_str(" AND event_type = ?");
        values.push(rusqlite::types::Value::Text(
            event_type.as_str().to_string(),
        ));
    }
    if let Some(success) = query.success {
        sql.push_str(" AND success = ?");
        values.push(rusqlite::types::Value::Integer(success as i64));
    }

    sql.push_str(" ORDER BY timestamp DESC, id DESC LIMIT ?");
    values.push(rusqlite::types::Value::Integer(query.limit as i64));

    let mut statement = connection
        .prepare(&sql)
        .map_err(|error| AuditLogError::Query(anyhow!("failed to prepare audit query: {error}")))?;
    let rows = statement
        .query_map(params_from_iter(values), |row| {
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
        })
        .map_err(|error| AuditLogError::Query(anyhow!("failed to execute audit query: {error}")))?;

    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|error| AuditLogError::Query(anyhow!("failed to decode audit rows: {error}")))
}

fn prune_expired_records(connection: &Connection, retention_days: u64) -> Result<usize> {
    let cutoff = Utc::now() - ChronoDuration::days(retention_days as i64);
    connection
        .execute(
            "DELETE FROM audit_log WHERE timestamp < ?1",
            params![cutoff.timestamp()],
        )
        .context("failed to prune expired audit records")
}

fn harden_audit_artifacts(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        ensure_directory_mode(parent, 0o700)?;
    }

    #[cfg(unix)]
    {
        for artifact in audit_artifact_paths(path) {
            if artifact.exists() {
                std::fs::set_permissions(&artifact, std::fs::Permissions::from_mode(0o600))
                    .with_context(|| format!("failed to chmod {}", artifact.display()))?;
            }
        }
    }

    #[cfg(not(unix))]
    {
        let _ = path;
    }

    Ok(())
}

#[cfg(unix)]
fn audit_artifact_paths(path: &Path) -> Vec<PathBuf> {
    let mut wal = path.as_os_str().to_os_string();
    wal.push("-wal");
    let mut shm = path.as_os_str().to_os_string();
    shm.push("-shm");
    vec![path.to_path_buf(), wal.into(), shm.into()]
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    use chrono::{Duration as ChronoDuration, Utc};
    use serde_json::json;
    use tokio::runtime::Runtime;

    use super::{AuditEventType, AuditLog, AuditLogError, AuditQuery, NewAuditEvent};

    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    fn unique_temp_dir(prefix: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be monotonic enough")
            .as_nanos();
        std::env::temp_dir().join(format!("{prefix}-{unique}"))
    }

    fn sample_config(db_path: PathBuf) -> nodelite_proto::AuditConfig {
        nodelite_proto::AuditConfig {
            enabled: true,
            db_path,
            retention_days: 90,
            log_successful_auth: true,
            log_failed_auth: true,
            log_token_events: true,
            log_rate_limit: true,
        }
    }

    #[test]
    fn audit_log_round_trips_and_filters_events() {
        let runtime = Runtime::new().expect("runtime should build");
        runtime.block_on(async {
            let temp_dir = unique_temp_dir("nodelite-audit-roundtrip");
            std::fs::create_dir_all(&temp_dir).expect("temp dir should exist");
            let db_path = temp_dir.join("audit.sqlite3");
            let audit = AuditLog::new(sample_config(db_path.clone()), 5);
            audit.initialize().await.expect("audit should initialize");

            let mut failure =
                NewAuditEvent::now(AuditEventType::LoginFailure, "198.51.100.10", false);
            failure.user = Some("viewer".to_string());
            failure.details = json!({"reason":"bad_basic_auth"});
            audit.record(failure).await.expect("failure should persist");

            let mut token =
                NewAuditEvent::now(AuditEventType::TokenInvalid, "198.51.100.11", false);
            token.node_id = Some("hk-01".to_string());
            token.details = json!({"reason":"expired"});
            audit
                .record(token)
                .await
                .expect("token event should persist");

            let all = audit
                .query(AuditQuery {
                    start: None,
                    end: None,
                    event_type: None,
                    success: None,
                    limit: 10,
                })
                .await
                .expect("audit query should succeed");
            assert_eq!(all.len(), 2);

            let filtered = audit
                .query(AuditQuery {
                    start: None,
                    end: None,
                    event_type: Some(AuditEventType::LoginFailure),
                    success: Some(false),
                    limit: 10,
                })
                .await
                .expect("filtered query should succeed");
            assert_eq!(filtered.len(), 1);
            assert_eq!(filtered[0].event_type, AuditEventType::LoginFailure);
            assert_eq!(filtered[0].user.as_deref(), Some("viewer"));

            let _ = std::fs::remove_file(&db_path);
            let _ = std::fs::remove_dir_all(&temp_dir);
        });
    }

    #[test]
    fn audit_log_prunes_records_older_than_retention_window() {
        let runtime = Runtime::new().expect("runtime should build");
        runtime.block_on(async {
            let temp_dir = unique_temp_dir("nodelite-audit-retention");
            std::fs::create_dir_all(&temp_dir).expect("temp dir should exist");
            let db_path = temp_dir.join("audit.sqlite3");
            let mut config = sample_config(db_path.clone());
            config.retention_days = 1;
            let audit = AuditLog::new(config, 5);
            audit.initialize().await.expect("audit should initialize");

            let old_event = NewAuditEvent {
                timestamp: Utc::now() - ChronoDuration::days(3),
                event_type: AuditEventType::LoginFailure,
                user: None,
                node_id: None,
                ip_address: "203.0.113.10".to_string(),
                user_agent: None,
                success: false,
                details: json!({"reason":"stale"}),
            };
            audit
                .record(old_event)
                .await
                .expect("old event should write");
            audit
                .record(NewAuditEvent::now(
                    AuditEventType::TotpVerifyFailure,
                    "203.0.113.11",
                    false,
                ))
                .await
                .expect("fresh event should write");

            let events = audit
                .query(AuditQuery {
                    start: None,
                    end: None,
                    event_type: None,
                    success: None,
                    limit: 10,
                })
                .await
                .expect("audit query should succeed");
            assert_eq!(events.len(), 1);
            assert_eq!(events[0].event_type, AuditEventType::TotpVerifyFailure);

            let _ = std::fs::remove_file(&db_path);
            let _ = std::fs::remove_dir_all(&temp_dir);
        });
    }

    #[test]
    #[cfg(unix)]
    fn audit_database_artifacts_are_mode_600() {
        let runtime = Runtime::new().expect("runtime should build");
        runtime.block_on(async {
            let temp_dir = unique_temp_dir("nodelite-audit-mode");
            std::fs::create_dir_all(&temp_dir).expect("temp dir should exist");
            let db_path = temp_dir.join("audit.sqlite3");
            let audit = AuditLog::new(sample_config(db_path.clone()), 5);
            audit.initialize().await.expect("audit should initialize");
            audit
                .record(NewAuditEvent::now(
                    AuditEventType::NodeConnected,
                    "198.51.100.20",
                    true,
                ))
                .await
                .expect("audit event should persist");

            let data_dir_mode = std::fs::metadata(&temp_dir)
                .expect("temp dir metadata")
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(data_dir_mode, 0o700);

            let db_mode = std::fs::metadata(&db_path)
                .expect("db metadata")
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(db_mode, 0o600);

            let _ = std::fs::remove_file(&db_path);
            let _ = std::fs::remove_dir_all(&temp_dir);
        });
    }

    #[test]
    fn disabled_audit_log_rejects_queries_but_ignores_records() {
        let runtime = Runtime::new().expect("runtime should build");
        runtime.block_on(async {
            let mut config = sample_config(PathBuf::from("/tmp/disabled-audit.sqlite3"));
            config.enabled = false;
            let audit = AuditLog::new(config, 5);

            audit
                .record(NewAuditEvent::now(
                    AuditEventType::LoginFailure,
                    "127.0.0.1",
                    false,
                ))
                .await
                .expect("disabled audit log should no-op on record");

            let error = audit
                .query(AuditQuery {
                    start: None,
                    end: None,
                    event_type: None,
                    success: None,
                    limit: 10,
                })
                .await
                .expect_err("disabled audit log should reject queries");
            assert!(matches!(error, AuditLogError::Disabled));
        });
    }
}
