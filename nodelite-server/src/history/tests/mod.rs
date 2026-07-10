//! Tests for history store writer, query, and throttling behavior.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::{DateTime, Duration, Utc};
use nodelite_proto::{
    HistoryPoint, LoadAverage, MemoryUsage, NetworkCounters, NodeIdentity, NodeSnapshot, NodeStatus,
};
use tokio::runtime::Runtime;

use super::{
    HISTORY_CHANNEL_CAPACITY, HISTORY_QUERY_SQL, HISTORY_READ_CACHE_KIB, HistoryError,
    HistoryQueryProbe, HistoryStore, SQLITE_BUSY_MAX_RETRIES, build_history_point,
    initialize_database, open_read_connection, query_history_between, sqlite_busy_retry_delay,
    write_history_point,
};

mod init_tests;
mod reader_tests;
mod writer_tests;

fn fake_status_for(node_id: &str, recorded_at: chrono::DateTime<Utc>) -> NodeStatus {
    NodeStatus {
        identity: NodeIdentity {
            node_id: node_id.to_string(),
            node_label: format!("{node_id}-label"),
            hostname: format!("{node_id}.internal"),
            os: "Ubuntu".to_string(),
            kernel_version: None,
            cpu_model: None,
            cpu_cores: 2,
            agent_version: "0.1.0".to_string(),
            boot_time: None,
            tags: Vec::new(),
        },
        remote_ip: Some("198.51.100.24".to_string()),
        geoip_country: None,
        geoip_city: None,
        geoip_latitude: None,
        geoip_longitude: None,
        location_override_country: None,
        location_override_city: None,
        location_override_latitude: None,
        location_override_longitude: None,
        snapshot: Some(NodeSnapshot {
            collected_at: recorded_at,
            cpu_usage_percent: Some(42.0),
            load: LoadAverage {
                one: 0.1,
                five: 0.2,
                fifteen: 0.3,
            },
            memory: MemoryUsage {
                total_bytes: 1024,
                used_bytes: 512,
                available_bytes: 512,
                swap_total_bytes: 0,
                swap_used_bytes: 0,
            },
            uptime_secs: 60,
            disks: Vec::new(),
            network: NetworkCounters {
                total_rx_bytes: 1,
                total_tx_bytes: 2,
                rx_bytes_per_sec: Some(3.0),
                tx_bytes_per_sec: Some(4.0),
                packet_loss_percent: Some(0.5),
            },
        }),
        last_seen: Some(recorded_at),
        latency_ms: Some(12),
        online: true,
    }
}

fn temp_history_db_path(test_name: &str) -> std::path::PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be monotonic enough")
        .as_nanos();
    let temp_dir = std::env::temp_dir().join(format!("nodelite-history-{test_name}-{unique}"));
    std::fs::create_dir_all(&temp_dir).expect("temp dir should exist");
    temp_dir.join("history.sqlite3")
}

#[cfg(unix)]
fn assert_mode_700(path: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt;

    let mode = std::fs::metadata(path)
        .expect("artifact metadata")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o700);
}

#[cfg(unix)]
fn assert_mode_600(path: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt;

    let mode = std::fs::metadata(path)
        .expect("artifact metadata")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o600);
}
