use std::path::PathBuf;

use super::{
    DEFAULT_ALERT_INSPECTION_CPU_WARN_PERCENT, DEFAULT_ALERT_INSPECTION_LATENCY_WARN_MS,
    DEFAULT_ALERT_INSPECTION_LOCAL_TIME, DEFAULT_ALERT_INSPECTION_LOOKBACK_HOURS,
    DEFAULT_ALERT_INSPECTION_MEMORY_WARN_PERCENT, DEFAULT_ALERT_INSPECTION_OFFLINE_GRACE_MINUTES,
    DEFAULT_ALERT_RULE_COOLDOWN_MINUTES, DEFAULT_ALERT_RULE_WINDOW_MINUTES,
    DEFAULT_AUDIT_RETENTION_DAYS, DEFAULT_CONNECT_TIMEOUT_SECS, DEFAULT_GEOIP_UPDATE_INTERVAL_DAYS,
    DEFAULT_HELLO_TIMEOUT_SECS, DEFAULT_INSECURE_TRANSPORT_WARN_INTERVAL_SECS,
    DEFAULT_MAX_INCOMING_MESSAGE_BYTES, DEFAULT_MAX_MESSAGE_BYTES, DEFAULT_MAX_OUTSTANDING_PINGS,
    DEFAULT_MAX_SANITIZED_DISKS, DEFAULT_MAX_SANITIZED_STRING_BYTES,
    DEFAULT_METRIC_ANOMALY_SESSION_LIMIT, DEFAULT_PING_INTERVAL_SECS,
    DEFAULT_REFRESH_INTERVAL_SECS, DEFAULT_REPORT_INTERVAL_SECS, DEFAULT_SQLITE_BUSY_TIMEOUT_SECS,
    DEFAULT_STALE_AFTER_SECS, DEFAULT_WS_AUTH_BLOCK_SECS, DEFAULT_WS_AUTH_FAIL_MAX_ATTEMPTS,
    DEFAULT_WS_AUTH_FAIL_WINDOW_SECS, DEFAULT_WS_MAX_CONNECTIONS_PER_IP,
    DEFAULT_WS_MAX_TOTAL_CONNECTIONS,
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

pub(super) fn default_trusted_proxies() -> Vec<String> {
    Vec::new()
}

pub(super) fn default_audit_db_path() -> PathBuf {
    PathBuf::from("./data/audit.sqlite3")
}

pub(super) fn default_geoip_database_path() -> PathBuf {
    PathBuf::from("./data/geoip/dbip.mmdb")
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

pub(super) fn default_metrics_export_node_resource_metrics() -> bool {
    false
}

pub(super) fn default_metrics_export_node_disk_metrics() -> bool {
    false
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

pub(super) fn default_geoip_enabled() -> bool {
    false
}

pub(super) fn default_geoip_provider() -> super::GeoIpProvider {
    super::GeoIpProvider::Dbip
}

pub(super) fn default_geoip_edition() -> super::GeoIpEdition {
    super::GeoIpEdition::CountryLite
}

pub(super) fn default_geoip_auto_update() -> bool {
    true
}

pub(super) fn default_geoip_update_interval_days() -> u64 {
    DEFAULT_GEOIP_UPDATE_INTERVAL_DAYS
}

pub(super) fn default_alert_rule_window_minutes() -> u64 {
    DEFAULT_ALERT_RULE_WINDOW_MINUTES
}

pub(super) fn default_alert_rule_cooldown_minutes() -> u64 {
    DEFAULT_ALERT_RULE_COOLDOWN_MINUTES
}

pub(super) fn default_alert_inspection_local_time() -> String {
    DEFAULT_ALERT_INSPECTION_LOCAL_TIME.to_string()
}

pub(super) fn default_alert_inspection_lookback_hours() -> u64 {
    DEFAULT_ALERT_INSPECTION_LOOKBACK_HOURS
}

pub(super) fn default_alert_inspection_offline_grace_minutes() -> u64 {
    DEFAULT_ALERT_INSPECTION_OFFLINE_GRACE_MINUTES
}

pub(super) fn default_alert_inspection_latency_warn_ms() -> u64 {
    DEFAULT_ALERT_INSPECTION_LATENCY_WARN_MS
}

pub(super) fn default_alert_inspection_cpu_warn_percent() -> u64 {
    DEFAULT_ALERT_INSPECTION_CPU_WARN_PERCENT
}

pub(super) fn default_alert_inspection_memory_warn_percent() -> u64 {
    DEFAULT_ALERT_INSPECTION_MEMORY_WARN_PERCENT
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
