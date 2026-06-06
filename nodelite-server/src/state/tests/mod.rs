//! Tests for shared state caching and registry lifecycle helpers.

use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use chrono::{Duration as ChronoDuration, TimeZone, Utc};
use nodelite_proto::{
    GeoIpLocation, LoadAverage, MemoryUsage, NetworkCounters, NodeIdentity, NodeSnapshot,
    ReadonlyAuthConfig, ServerConfig, WsConfig, percentage,
};

use super::{Registry, SessionControlHandle, SharedState};

mod overview_tests;
mod registry_tests;
mod view_cache_tests;

fn sample_identity() -> NodeIdentity {
    NodeIdentity {
        node_id: "hk-01".to_string(),
        node_label: "Hong Kong 01".to_string(),
        hostname: "hk-01".to_string(),
        os: "linux".to_string(),
        kernel_version: None,
        cpu_model: None,
        cpu_cores: 4,
        agent_version: "0.1.0".to_string(),
        boot_time: None,
        tags: Vec::new(),
    }
}

fn sample_config() -> ServerConfig {
    ServerConfig {
        listen: SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 8080)),
        public_base_url: "http://127.0.0.1:8080".to_string(),
        insecure_allow_http: false,
        trusted_proxies: Vec::new(),
        readonly_auth: Some(ReadonlyAuthConfig {
            username: "viewer".to_string(),
            password: "secret".to_string(),
            enable_2fa: false,
            totp_secret: None,
        }),
        ws: WsConfig {
            max_total_connections: 128,
            max_connections_per_ip: 64,
            auth_fail_window_secs: 300,
            auth_fail_max_attempts: 12,
            auth_block_secs: 900,
        },
        metrics: nodelite_proto::MetricsConfig::default(),
        audit: nodelite_proto::AuditConfig {
            enabled: true,
            db_path: PathBuf::from("/tmp/nodelite-test-audit.sqlite3"),
            retention_days: 90,
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
        node_registry_path: PathBuf::from("/tmp/nodelite-test-registry.json"),
        history_db_path: PathBuf::from("/tmp/nodelite-test-history.sqlite3"),
        snapshot_path: PathBuf::from("/tmp/nodelite-test-snapshot.json"),
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
    }
}

fn sample_snapshot(now: chrono::DateTime<Utc>) -> NodeSnapshot {
    NodeSnapshot {
        collected_at: now,
        cpu_usage_percent: Some(percentage(1, 2)),
        load: LoadAverage {
            one: 0.1,
            five: 0.2,
            fifteen: 0.3,
        },
        memory: MemoryUsage {
            total_bytes: 1024,
            used_bytes: 512,
            available_bytes: 256,
            swap_total_bytes: 128,
            swap_used_bytes: 64,
        },
        uptime_secs: 60,
        disks: Vec::new(),
        network: NetworkCounters {
            total_rx_bytes: 100,
            total_tx_bytes: 200,
            rx_bytes_per_sec: Some(5.0),
            tx_bytes_per_sec: Some(7.0),
        },
    }
}

fn sample_disk_usage() -> nodelite_proto::DiskUsage {
    nodelite_proto::DiskUsage {
        device: "/dev/vda1".to_string(),
        mount_point: "/".to_string(),
        fs_type: "ext4".to_string(),
        total_bytes: 1024,
        available_bytes: 512,
        used_bytes: 512,
        used_percent: 50.0,
    }
}
