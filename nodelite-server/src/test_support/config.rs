//! Test-only server and WebSocket configuration fixtures.

use std::net::SocketAddr;
use std::path::PathBuf;

use nodelite_proto::{AuditConfig, ReadonlyAuthConfig, ServerConfig, WsConfig};

pub(crate) fn test_ws_config(
    max_total_connections: usize,
    max_connections_per_ip: usize,
) -> WsConfig {
    WsConfig {
        max_total_connections,
        max_connections_per_ip,
        auth_fail_window_secs: 300,
        auth_fail_max_attempts: 12,
        auth_block_secs: 900,
    }
}

pub(crate) fn test_server_config(
    listen: SocketAddr,
    public_base_url: String,
    registry_path: PathBuf,
    history_path: PathBuf,
    snapshot_path: PathBuf,
) -> ServerConfig {
    ServerConfig {
        listen,
        public_base_url,
        insecure_allow_http: false,
        trusted_proxies: Vec::new(),
        readonly_auth: Some(ReadonlyAuthConfig {
            username: "viewer".to_string(),
            password: "secret".to_string(),
            enable_2fa: false,
            totp_secret: None,
        }),
        ws: test_ws_config(128, 128),
        metrics: nodelite_proto::MetricsConfig::default(),
        audit: AuditConfig {
            enabled: true,
            db_path: history_path.with_file_name("audit.sqlite3"),
            retention_days: 90,
            writer_batch_max: nodelite_proto::DEFAULT_AUDIT_WRITER_BATCH_MAX,
            writer_flush_interval_ms: nodelite_proto::DEFAULT_AUDIT_WRITER_FLUSH_INTERVAL_MS,
            log_successful_auth: true,
            log_failed_auth: true,
            log_token_events: true,
            log_rate_limit: true,
        },
        geoip: nodelite_proto::GeoIpConfig {
            enabled: false,
            provider: nodelite_proto::GeoIpProvider::Dbip,
            edition: nodelite_proto::GeoIpEdition::CountryLite,
            database_path: PathBuf::from("./data/geoip/dbip.mmdb"),
            auto_update: true,
            update_interval_days: nodelite_proto::DEFAULT_GEOIP_UPDATE_INTERVAL_DAYS,
        },
        alerting: nodelite_proto::AlertingConfig::default(),
        node_registry_path: registry_path,
        history_db_path: history_path,
        history_query_concurrency: nodelite_proto::DEFAULT_HISTORY_QUERY_CONCURRENCY,
        history_read_cache_kib: nodelite_proto::DEFAULT_HISTORY_READ_CACHE_KIB,
        history_writer_batch_max: nodelite_proto::DEFAULT_HISTORY_WRITER_BATCH_MAX,
        history_writer_flush_interval_ms: nodelite_proto::DEFAULT_HISTORY_WRITER_FLUSH_INTERVAL_MS,
        snapshot_path,
        stale_after_secs: 5,
        ping_interval_secs: 60,
        max_message_bytes: 64 * 1024,
        refresh_interval_secs: 5,
        ignored_filesystems: vec!["tmpfs".to_string(), "devtmpfs".to_string()],
        agent_release_base_url: None,
        agent_release_sha256_x86_64: None,
        agent_release_sha256_aarch64: None,
        hello_timeout_secs: 10,
        max_outstanding_pings: 32,
        insecure_transport_warn_interval_secs: 900,
        max_sanitized_disks: 64,
        max_sanitized_string_bytes: 256,
        metric_anomaly_session_limit: 5,
        sqlite_busy_timeout_secs: 5,
        token_verify_max_parallelism: nodelite_proto::DEFAULT_TOKEN_VERIFY_MAX_PARALLELISM,
    }
}
