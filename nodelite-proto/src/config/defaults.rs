//! Defaults are shared by parsed and generated config; server-only defaults stay feature gated.

#[cfg(feature = "server-config")]
use std::path::PathBuf;

use super::{
    DEFAULT_AGENT_AUTH_TIMEOUT_SECS, DEFAULT_AGENT_INBOUND_TIMEOUT_SECS,
    DEFAULT_AGENT_SEND_TIMEOUT_SECS, DEFAULT_CONNECT_TIMEOUT_SECS,
    DEFAULT_INSECURE_TRANSPORT_WARN_INTERVAL_SECS, DEFAULT_MAX_INCOMING_MESSAGE_BYTES,
    DEFAULT_REPORT_INTERVAL_SECS,
};
#[cfg(feature = "server-config")]
use super::{
    DEFAULT_ALERT_INSPECTION_CPU_WARN_PERCENT, DEFAULT_ALERT_INSPECTION_LATENCY_WARN_MS,
    DEFAULT_ALERT_INSPECTION_LOCAL_TIME, DEFAULT_ALERT_INSPECTION_LOOKBACK_HOURS,
    DEFAULT_ALERT_INSPECTION_MEMORY_WARN_PERCENT, DEFAULT_ALERT_INSPECTION_OFFLINE_GRACE_MINUTES,
    DEFAULT_ALERT_RULE_COOLDOWN_MINUTES, DEFAULT_ALERT_RULE_WINDOW_MINUTES,
    DEFAULT_AUDIT_RETENTION_DAYS, DEFAULT_AUDIT_WRITER_BATCH_MAX,
    DEFAULT_AUDIT_WRITER_FLUSH_INTERVAL_MS, DEFAULT_GEOIP_UPDATE_INTERVAL_DAYS,
    DEFAULT_HELLO_TIMEOUT_SECS, DEFAULT_HISTORY_QUERY_CONCURRENCY, DEFAULT_HISTORY_READ_CACHE_KIB,
    DEFAULT_HISTORY_WRITER_BATCH_MAX, DEFAULT_HISTORY_WRITER_FLUSH_INTERVAL_MS,
    DEFAULT_MAX_MESSAGE_BYTES, DEFAULT_MAX_OUTSTANDING_PINGS, DEFAULT_MAX_SANITIZED_DISKS,
    DEFAULT_MAX_SANITIZED_STRING_BYTES, DEFAULT_METRIC_ANOMALY_SESSION_LIMIT,
    DEFAULT_PING_INTERVAL_SECS, DEFAULT_REFRESH_INTERVAL_SECS, DEFAULT_SQLITE_BUSY_TIMEOUT_SECS,
    DEFAULT_STALE_AFTER_SECS, DEFAULT_TOKEN_VERIFY_MAX_PARALLELISM, DEFAULT_WS_AUTH_BLOCK_SECS,
    DEFAULT_WS_AUTH_FAIL_MAX_ATTEMPTS, DEFAULT_WS_AUTH_FAIL_WINDOW_SECS,
    DEFAULT_WS_MAX_CONNECTIONS_PER_IP, DEFAULT_WS_MAX_TOTAL_CONNECTIONS,
};

default_fns! {
    default_report_interval_secs -> u64 = DEFAULT_REPORT_INTERVAL_SECS;
    default_insecure_transport_warn_interval_secs -> u64 = DEFAULT_INSECURE_TRANSPORT_WARN_INTERVAL_SECS;
    default_connect_timeout_secs -> u64 = DEFAULT_CONNECT_TIMEOUT_SECS;
    default_max_incoming_message_bytes -> usize = DEFAULT_MAX_INCOMING_MESSAGE_BYTES;
    default_agent_auth_timeout_secs -> u64 = DEFAULT_AGENT_AUTH_TIMEOUT_SECS;
    default_agent_send_timeout_secs -> u64 = DEFAULT_AGENT_SEND_TIMEOUT_SECS;
    default_agent_inbound_timeout_secs -> u64 = DEFAULT_AGENT_INBOUND_TIMEOUT_SECS;
}

#[cfg(feature = "server-config")]
default_fns! {
    default_history_db_path -> PathBuf = PathBuf::from("./data/history.sqlite3");
    default_history_query_concurrency -> usize = DEFAULT_HISTORY_QUERY_CONCURRENCY;
    default_history_read_cache_kib -> u64 = DEFAULT_HISTORY_READ_CACHE_KIB;
    default_history_writer_batch_max -> usize = DEFAULT_HISTORY_WRITER_BATCH_MAX;
    default_history_writer_flush_interval_ms -> u64 = DEFAULT_HISTORY_WRITER_FLUSH_INTERVAL_MS;
    default_node_registry_path -> PathBuf = PathBuf::from("./config/server.json");
    default_snapshot_path -> PathBuf = PathBuf::from("./data/snapshot.json");
    default_trusted_proxies -> Vec<String> = Vec::new();
    default_audit_db_path -> PathBuf = PathBuf::from("./data/audit.sqlite3");
    default_geoip_database_path -> PathBuf = PathBuf::from("./data/geoip/dbip.mmdb");
    default_stale_after_secs -> u64 = DEFAULT_STALE_AFTER_SECS;
    default_ping_interval_secs -> u64 = DEFAULT_PING_INTERVAL_SECS;
    default_max_message_bytes -> usize = DEFAULT_MAX_MESSAGE_BYTES;
    default_refresh_interval_secs -> u64 = DEFAULT_REFRESH_INTERVAL_SECS;
    default_ws_max_total_connections -> usize = DEFAULT_WS_MAX_TOTAL_CONNECTIONS;
    default_ws_max_connections_per_ip -> usize = DEFAULT_WS_MAX_CONNECTIONS_PER_IP;
    default_ws_auth_fail_window_secs -> u64 = DEFAULT_WS_AUTH_FAIL_WINDOW_SECS;
    default_ws_auth_fail_max_attempts -> usize = DEFAULT_WS_AUTH_FAIL_MAX_ATTEMPTS;
    default_ws_auth_block_secs -> u64 = DEFAULT_WS_AUTH_BLOCK_SECS;
    default_hello_timeout_secs -> u64 = DEFAULT_HELLO_TIMEOUT_SECS;
    default_max_outstanding_pings -> usize = DEFAULT_MAX_OUTSTANDING_PINGS;
    default_max_sanitized_disks -> usize = DEFAULT_MAX_SANITIZED_DISKS;
    default_max_sanitized_string_bytes -> usize = DEFAULT_MAX_SANITIZED_STRING_BYTES;
    default_metric_anomaly_session_limit -> usize = DEFAULT_METRIC_ANOMALY_SESSION_LIMIT;
    default_sqlite_busy_timeout_secs -> u64 = DEFAULT_SQLITE_BUSY_TIMEOUT_SECS;
    default_token_verify_max_parallelism -> usize = DEFAULT_TOKEN_VERIFY_MAX_PARALLELISM;
    default_metrics_export_node_resource_metrics -> bool = false;
    default_metrics_export_node_disk_metrics -> bool = false;
    default_audit_enabled -> bool = true;
    default_audit_retention_days -> u64 = DEFAULT_AUDIT_RETENTION_DAYS;
    default_audit_writer_batch_max -> usize = DEFAULT_AUDIT_WRITER_BATCH_MAX;
    default_audit_writer_flush_interval_ms -> u64 = DEFAULT_AUDIT_WRITER_FLUSH_INTERVAL_MS;
    default_audit_log_successful_auth -> bool = true;
    default_audit_log_failed_auth -> bool = true;
    default_audit_log_token_events -> bool = true;
    default_audit_log_rate_limit -> bool = true;
    default_geoip_enabled -> bool = true;
    default_geoip_provider -> super::GeoIpProvider = super::GeoIpProvider::Ipwhois;
    default_geoip_edition -> super::GeoIpEdition = super::GeoIpEdition::CountryLite;
    default_geoip_auto_update -> bool = false;
    default_geoip_update_interval_days -> u64 = DEFAULT_GEOIP_UPDATE_INTERVAL_DAYS;
    default_alert_rule_window_minutes -> u64 = DEFAULT_ALERT_RULE_WINDOW_MINUTES;
    default_alert_rule_cooldown_minutes -> u64 = DEFAULT_ALERT_RULE_COOLDOWN_MINUTES;
    default_alert_inspection_local_time -> String = DEFAULT_ALERT_INSPECTION_LOCAL_TIME.to_string();
    default_alert_inspection_lookback_hours -> u64 = DEFAULT_ALERT_INSPECTION_LOOKBACK_HOURS;
    default_alert_inspection_offline_grace_minutes -> u64 = DEFAULT_ALERT_INSPECTION_OFFLINE_GRACE_MINUTES;
    default_alert_inspection_latency_warn_ms -> u64 = DEFAULT_ALERT_INSPECTION_LATENCY_WARN_MS;
    default_alert_inspection_cpu_warn_percent -> u64 = DEFAULT_ALERT_INSPECTION_CPU_WARN_PERCENT;
    default_alert_inspection_memory_warn_percent -> u64 = DEFAULT_ALERT_INSPECTION_MEMORY_WARN_PERCENT;
    default_ignored_filesystems -> Vec<String> = ["devtmpfs", "overlay", "tmpfs"].map(str::to_string).to_vec();
}
