use std::path::PathBuf;

use super::{
    DEFAULT_AUDIT_RETENTION_DAYS, DEFAULT_CONNECT_TIMEOUT_SECS, DEFAULT_HELLO_TIMEOUT_SECS,
    DEFAULT_INSECURE_TRANSPORT_WARN_INTERVAL_SECS, DEFAULT_MAX_INCOMING_MESSAGE_BYTES,
    DEFAULT_MAX_MESSAGE_BYTES, DEFAULT_MAX_OUTSTANDING_PINGS, DEFAULT_MAX_SANITIZED_DISKS,
    DEFAULT_MAX_SANITIZED_STRING_BYTES, DEFAULT_METRIC_ANOMALY_SESSION_LIMIT,
    DEFAULT_PING_INTERVAL_SECS, DEFAULT_REFRESH_INTERVAL_SECS, DEFAULT_REPORT_INTERVAL_SECS,
    DEFAULT_SQLITE_BUSY_TIMEOUT_SECS, DEFAULT_STALE_AFTER_SECS, DEFAULT_WS_AUTH_BLOCK_SECS,
    DEFAULT_WS_AUTH_FAIL_MAX_ATTEMPTS, DEFAULT_WS_AUTH_FAIL_WINDOW_SECS,
    DEFAULT_WS_MAX_CONNECTIONS_PER_IP, DEFAULT_WS_MAX_TOTAL_CONNECTIONS,
};

pub(super) fn default_history_db_path() -> PathBuf {
    PathBuf::from("./data/history.sqlite3")
}

pub(super) fn default_node_registry_path() -> PathBuf {
    PathBuf::from("./config/server.json")
}

pub(super) fn default_snapshot_path() -> PathBuf {
    PathBuf::from("./data/snapshot.json")
}

pub(super) fn default_audit_db_path() -> PathBuf {
    PathBuf::from("./data/audit.sqlite3")
}

pub(super) fn default_stale_after_secs() -> u64 {
    DEFAULT_STALE_AFTER_SECS
}

pub(super) fn default_ping_interval_secs() -> u64 {
    DEFAULT_PING_INTERVAL_SECS
}

pub(super) fn default_max_message_bytes() -> usize {
    DEFAULT_MAX_MESSAGE_BYTES
}

pub(super) fn default_refresh_interval_secs() -> u64 {
    DEFAULT_REFRESH_INTERVAL_SECS
}

pub(super) fn default_ws_max_total_connections() -> usize {
    DEFAULT_WS_MAX_TOTAL_CONNECTIONS
}

pub(super) fn default_ws_max_connections_per_ip() -> usize {
    DEFAULT_WS_MAX_CONNECTIONS_PER_IP
}

pub(super) fn default_ws_auth_fail_window_secs() -> u64 {
    DEFAULT_WS_AUTH_FAIL_WINDOW_SECS
}

pub(super) fn default_ws_auth_fail_max_attempts() -> usize {
    DEFAULT_WS_AUTH_FAIL_MAX_ATTEMPTS
}

pub(super) fn default_ws_auth_block_secs() -> u64 {
    DEFAULT_WS_AUTH_BLOCK_SECS
}

pub(super) fn default_report_interval_secs() -> u64 {
    DEFAULT_REPORT_INTERVAL_SECS
}

pub(super) fn default_hello_timeout_secs() -> u64 {
    DEFAULT_HELLO_TIMEOUT_SECS
}

pub(super) fn default_max_outstanding_pings() -> usize {
    DEFAULT_MAX_OUTSTANDING_PINGS
}

pub(super) fn default_insecure_transport_warn_interval_secs() -> u64 {
    DEFAULT_INSECURE_TRANSPORT_WARN_INTERVAL_SECS
}

pub(super) fn default_max_sanitized_disks() -> usize {
    DEFAULT_MAX_SANITIZED_DISKS
}

pub(super) fn default_max_sanitized_string_bytes() -> usize {
    DEFAULT_MAX_SANITIZED_STRING_BYTES
}

pub(super) fn default_metric_anomaly_session_limit() -> usize {
    DEFAULT_METRIC_ANOMALY_SESSION_LIMIT
}

pub(super) fn default_sqlite_busy_timeout_secs() -> u64 {
    DEFAULT_SQLITE_BUSY_TIMEOUT_SECS
}

pub(super) fn default_audit_enabled() -> bool {
    true
}

pub(super) fn default_audit_retention_days() -> u64 {
    DEFAULT_AUDIT_RETENTION_DAYS
}

pub(super) fn default_audit_log_successful_auth() -> bool {
    true
}

pub(super) fn default_audit_log_failed_auth() -> bool {
    true
}

pub(super) fn default_audit_log_token_events() -> bool {
    true
}

pub(super) fn default_audit_log_rate_limit() -> bool {
    true
}

pub(super) fn default_connect_timeout_secs() -> u64 {
    DEFAULT_CONNECT_TIMEOUT_SECS
}

pub(super) fn default_max_incoming_message_bytes() -> usize {
    DEFAULT_MAX_INCOMING_MESSAGE_BYTES
}

/// 默认忽略的文件系统类型;这些通常是虚拟挂载,不应在磁盘视图中出现。
pub(super) fn default_ignored_filesystems() -> Vec<String> {
    vec![
        "devtmpfs".to_string(),
        "overlay".to_string(),
        "tmpfs".to_string(),
    ]
}
