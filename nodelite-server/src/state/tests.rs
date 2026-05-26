//! Tests for shared state caching and registry lifecycle helpers.

use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use chrono::{Duration as ChronoDuration, TimeZone, Utc};
use nodelite_proto::{
    LoadAverage, MemoryUsage, NetworkCounters, NodeIdentity, NodeSnapshot, ReadonlyAuthConfig,
    ServerConfig, WsConfig, percentage,
};

use super::{Registry, SessionControlHandle, SharedState};

#[test]
fn newer_session_replaces_older_one() {
    let mut registry = Registry::default();
    let now = Utc
        .with_ymd_and_hms(2026, 5, 7, 0, 0, 0)
        .single()
        .expect("valid test datetime");
    let identity = NodeIdentity {
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
    };

    registry.register_node(1, identity.clone(), Some("198.51.100.10".to_string()), now);
    registry.register_node(
        2,
        identity,
        Some("198.51.100.11".to_string()),
        now + ChronoDuration::seconds(3),
    );

    assert!(
        registry
            .update_snapshot("hk-01", 1, sample_snapshot(now), now)
            .is_none()
    );
    assert!(
        registry
            .update_snapshot(
                "hk-01",
                2,
                sample_snapshot(now + ChronoDuration::seconds(4)),
                now,
            )
            .is_some()
    );
}

#[test]
fn stale_nodes_are_marked_offline() {
    let mut registry = Registry::default();
    let now = Utc
        .with_ymd_and_hms(2026, 5, 7, 0, 0, 0)
        .single()
        .expect("valid test datetime");

    registry.register_node(7, sample_identity(), Some("198.51.100.10".to_string()), now);
    assert_eq!(
        registry.mark_stale(Duration::from_secs(10), now + ChronoDuration::seconds(15)),
        1
    );
    assert!(
        !registry
            .list_statuses()
            .first()
            .expect("node status")
            .online
    );
}

#[test]
fn overview_saturates_totals_and_skips_invalid_rates() {
    let mut registry = Registry::default();
    let now = Utc
        .with_ymd_and_hms(2026, 5, 7, 0, 0, 0)
        .single()
        .expect("valid test datetime");

    registry.register_node(1, sample_identity(), Some("198.51.100.10".to_string()), now);
    registry.register_node(
        2,
        NodeIdentity {
            node_id: "sg-01".to_string(),
            node_label: "Singapore 01".to_string(),
            ..sample_identity()
        },
        Some("198.51.100.11".to_string()),
        now,
    );

    let mut first = sample_snapshot(now);
    first.network.total_rx_bytes = u64::MAX;
    first.network.total_tx_bytes = u64::MAX;
    first.network.rx_bytes_per_sec = Some(f64::INFINITY);
    first.network.tx_bytes_per_sec = Some(1.5);
    registry.update_snapshot("hk-01", 1, first, now);

    let mut second = sample_snapshot(now);
    second.network.total_rx_bytes = 42;
    second.network.total_tx_bytes = 99;
    second.network.rx_bytes_per_sec = Some(2.5);
    second.network.tx_bytes_per_sec = Some(-10.0);
    registry.update_snapshot("sg-01", 2, second, now);

    let overview = registry.overview();
    assert_eq!(overview.total_rx_bytes, u64::MAX);
    assert_eq!(overview.total_tx_bytes, u64::MAX);
    assert_eq!(overview.current_rx_bytes_per_sec, 2.5);
    assert_eq!(overview.current_tx_bytes_per_sec, 1.5);
}

#[test]
fn overview_avoids_overflow_when_summing_latency() {
    // 用接近 u64::MAX 的延迟值复现"原始 sum::<u64>() 会溢出"的场景:
    // 旧实现在 debug 构建下 panic,release 构建下回绕成异常小的平均值。
    let mut registry = Registry::default();
    let now = Utc
        .with_ymd_and_hms(2026, 5, 7, 0, 0, 0)
        .single()
        .expect("valid test datetime");

    registry.register_node(1, sample_identity(), Some("198.51.100.10".to_string()), now);
    registry.register_node(
        2,
        NodeIdentity {
            node_id: "sg-01".to_string(),
            node_label: "Singapore 01".to_string(),
            ..sample_identity()
        },
        Some("198.51.100.11".to_string()),
        now,
    );

    registry.update_snapshot("hk-01", 1, sample_snapshot(now), now);
    registry.update_snapshot("sg-01", 2, sample_snapshot(now), now);
    registry.update_latency("hk-01", 1, u64::MAX / 2 + 1, now);
    registry.update_latency("sg-01", 2, u64::MAX / 2 + 1, now);

    let overview = registry.overview();
    let average = overview
        .average_latency_ms
        .expect("average latency should be reported");
    assert!(average.is_finite());
    assert!(average > (u64::MAX as f64) / 4.0);
}

#[test]
fn session_control_is_only_available_for_current_online_session() {
    let mut registry = Registry::default();
    let now = Utc
        .with_ymd_and_hms(2026, 5, 7, 0, 0, 0)
        .single()
        .expect("valid test datetime");
    registry.register_node(7, sample_identity(), Some("198.51.100.10".to_string()), now);

    let (control, _control_rx) = SessionControlHandle::channel();
    assert!(registry.attach_session_control("hk-01", 7, control));
    assert!(registry.session_control("hk-01").is_some());

    registry.register_node(
        8,
        sample_identity(),
        Some("198.51.100.11".to_string()),
        now + ChronoDuration::seconds(1),
    );
    assert!(
        registry.session_control("hk-01").is_none(),
        "newer session should clear the previous control handle",
    );
}

#[test]
fn mark_disconnected_clears_session_control() {
    let mut registry = Registry::default();
    let now = Utc
        .with_ymd_and_hms(2026, 5, 7, 0, 0, 0)
        .single()
        .expect("valid test datetime");
    registry.register_node(9, sample_identity(), Some("198.51.100.10".to_string()), now);

    let (control, _control_rx) = SessionControlHandle::channel();
    assert!(registry.attach_session_control("hk-01", 9, control));
    registry.mark_disconnected("hk-01", 9);

    assert!(registry.session_control("hk-01").is_none());
}

#[tokio::test]
async fn cached_api_json_invalidates_after_visible_status_change() {
    let shared = SharedState::new(Arc::new(sample_config()));
    let session_id = shared
        .register_node(sample_identity(), Some("198.51.100.10".to_string()))
        .await;

    let first_nodes = shared.nodes_json_bytes().await.expect("nodes json");
    let first_overview = shared.overview_json_bytes().await.expect("overview json");
    assert_eq!(shared.api_nodes_cache_build_count(), 1);
    assert_eq!(shared.api_overview_cache_build_count(), 1);

    shared.mark_disconnected("hk-01", session_id).await;

    let second_overview = shared
        .overview_json_bytes()
        .await
        .expect("overview json after disconnect");
    assert_eq!(shared.api_nodes_cache_build_count(), 1);
    assert_eq!(shared.api_overview_cache_build_count(), 2);

    let second_nodes = shared
        .nodes_json_bytes()
        .await
        .expect("nodes json after disconnect");
    assert_eq!(shared.api_nodes_cache_build_count(), 2);
    assert_eq!(shared.api_overview_cache_build_count(), 2);

    assert_ne!(first_nodes, second_nodes);
    assert_ne!(first_overview, second_overview);
    assert!(
        std::str::from_utf8(&second_nodes)
            .expect("utf8")
            .contains("\"online\":false")
    );
}

#[tokio::test]
async fn concurrent_api_cache_miss_serializes_once() {
    let shared = SharedState::new(Arc::new(sample_config()));
    shared
        .register_node(sample_identity(), Some("198.51.100.10".to_string()))
        .await;

    let mut tasks = Vec::new();
    for _ in 0..10 {
        let shared = shared.clone();
        tasks.push(tokio::spawn(async move {
            shared.nodes_json_bytes().await.expect("nodes json")
        }));
    }

    let mut first = None;
    for task in tasks {
        let body = task.await.expect("task join");
        if let Some(previous) = first.as_ref() {
            assert_eq!(previous, &body);
        } else {
            first = Some(body);
        }
    }

    assert_eq!(shared.api_cache_build_count(), 1);
}

#[tokio::test]
async fn api_overview_and_nodes_caches_build_independently() {
    let shared = SharedState::new(Arc::new(sample_config()));
    shared
        .register_node(sample_identity(), Some("198.51.100.10".to_string()))
        .await;

    let first_overview = shared.overview_json_bytes().await.expect("overview json");
    assert_eq!(shared.api_overview_cache_build_count(), 1);
    assert_eq!(
        shared.api_nodes_cache_build_count(),
        0,
        "overview miss must not serialize or populate the nodes body",
    );

    let cached_overview = shared.overview_json_bytes().await.expect("overview json");
    assert_eq!(first_overview, cached_overview);
    assert_eq!(shared.api_overview_cache_build_count(), 1);
    assert_eq!(shared.api_nodes_cache_build_count(), 0);
    let metrics = shared.api_cache_metrics();
    assert_eq!(metrics.overview_hits, 1);
    assert_eq!(metrics.overview_misses, 1);
    assert!(metrics.overview_body_bytes > 0);
    assert_eq!(metrics.nodes_hits, 0);
    assert_eq!(metrics.nodes_misses, 0);
    assert_eq!(metrics.nodes_body_bytes, 0);

    let first_nodes = shared.nodes_json_bytes().await.expect("nodes json");
    assert_eq!(shared.api_overview_cache_build_count(), 1);
    assert_eq!(shared.api_nodes_cache_build_count(), 1);

    let cached_nodes = shared.nodes_json_bytes().await.expect("nodes json");
    assert_eq!(first_nodes, cached_nodes);
    assert_eq!(shared.api_overview_cache_build_count(), 1);
    assert_eq!(shared.api_nodes_cache_build_count(), 1);
    let metrics = shared.api_cache_metrics();
    assert_eq!(metrics.overview_hits, 1);
    assert_eq!(metrics.overview_misses, 1);
    assert_eq!(metrics.nodes_hits, 1);
    assert_eq!(metrics.nodes_misses, 1);
    assert!(metrics.nodes_body_bytes > 0);
    assert!(metrics.overview_body_bytes > 0);
}

#[tokio::test]
async fn registry_disk_entries_total_counts_snapshot_disks() {
    let shared = SharedState::new(Arc::new(sample_config()));
    let first_session = shared
        .register_node(sample_identity(), Some("198.51.100.10".to_string()))
        .await;
    let second_session = shared
        .register_node(
            NodeIdentity {
                node_id: "sg-01".to_string(),
                node_label: "Singapore 01".to_string(),
                ..sample_identity()
            },
            Some("198.51.100.11".to_string()),
        )
        .await;

    let mut first = sample_snapshot(Utc::now());
    first.disks.resize_with(2, sample_disk_usage);
    let mut second = sample_snapshot(Utc::now());
    second.disks.resize_with(3, sample_disk_usage);

    assert!(
        shared
            .update_snapshot("hk-01", first_session, first)
            .await
            .is_some()
    );
    assert!(
        shared
            .update_snapshot("sg-01", second_session, second)
            .await
            .is_some()
    );

    assert_eq!(shared.registry_disk_entries_total().await, 5);
}

#[tokio::test]
async fn snapshot_update_only_invalidates_nodes_view() {
    let shared = SharedState::new(Arc::new(sample_config()));
    let readiness = crate::ServerReadiness::new(true);
    let session_id = shared
        .register_node(sample_identity(), Some("198.51.100.10".to_string()))
        .await;

    // Prime每个视图各一次。
    let _ = shared.overview_json_bytes().await.expect("overview json");
    let _ = shared.nodes_json_bytes().await.expect("nodes json");
    let _ = shared.metrics_text(&readiness).await;
    assert_eq!(shared.api_overview_cache_build_count(), 1);
    assert_eq!(shared.api_nodes_cache_build_count(), 1);
    assert_eq!(shared.metrics_cache_build_count(), 1);

    // 单纯的 snapshot 更新只让 nodes 视图失效;overview/metrics 仍命中缓存。
    assert!(
        shared
            .update_snapshot("hk-01", session_id, sample_snapshot(Utc::now()))
            .await
            .is_some()
    );
    let _ = shared.overview_json_bytes().await.expect("overview cached");
    let _ = shared.metrics_text(&readiness).await;
    assert_eq!(shared.api_overview_cache_build_count(), 1);
    assert_eq!(shared.metrics_cache_build_count(), 1);

    let _ = shared.nodes_json_bytes().await.expect("nodes rebuilds");
    assert_eq!(shared.api_nodes_cache_build_count(), 2);

    // latency 更新同样只触达 nodes。
    assert!(shared.update_latency("hk-01", session_id, 42).await);
    let _ = shared.overview_json_bytes().await.expect("overview cached");
    let _ = shared.metrics_text(&readiness).await;
    assert_eq!(shared.api_overview_cache_build_count(), 1);
    assert_eq!(shared.metrics_cache_build_count(), 1);
    let _ = shared
        .nodes_json_bytes()
        .await
        .expect("nodes rebuilds again");
    assert_eq!(shared.api_nodes_cache_build_count(), 3);

    // 真正的结构性变更仍然连带使三视图失效。
    shared.mark_disconnected("hk-01", session_id).await;
    let _ = shared
        .overview_json_bytes()
        .await
        .expect("overview rebuilds");
    let _ = shared.metrics_text(&readiness).await;
    let _ = shared.nodes_json_bytes().await.expect("nodes rebuilds");
    assert_eq!(shared.api_overview_cache_build_count(), 2);
    assert_eq!(shared.metrics_cache_build_count(), 2);
    assert_eq!(shared.api_nodes_cache_build_count(), 4);
}

#[tokio::test]
async fn metrics_cache_reuses_and_invalidates_cleanly() {
    let shared = SharedState::new(Arc::new(sample_config()));
    let readiness = crate::ServerReadiness::new(true);
    let session_id = shared
        .register_node(sample_identity(), Some("198.51.100.10".to_string()))
        .await;
    assert!(
        shared
            .update_snapshot("hk-01", session_id, sample_snapshot(Utc::now()))
            .await
            .is_some()
    );

    let mut tasks = Vec::new();
    for _ in 0..10 {
        let shared = shared.clone();
        let readiness = readiness.clone();
        tasks.push(tokio::spawn(async move {
            shared.metrics_text(&readiness).await
        }));
    }

    let mut first = None;
    for task in tasks {
        let body = task.await.expect("task join");
        if let Some(previous) = first.as_ref() {
            assert_eq!(previous, &body);
        } else {
            first = Some(body);
        }
    }
    assert_eq!(shared.metrics_cache_build_count(), 1);

    let cached = shared.metrics_text(&readiness).await;
    assert_eq!(shared.metrics_cache_build_count(), 1);
    assert_eq!(first.expect("first metrics body"), cached);

    shared.mark_disconnected("hk-01", session_id).await;
    let after_disconnect = shared.metrics_text(&readiness).await;
    assert_eq!(shared.metrics_cache_build_count(), 2);
    assert_ne!(cached, after_disconnect);

    readiness.mark_history_available(false);
    let after_readiness = shared.metrics_text(&readiness).await;
    assert_eq!(shared.metrics_cache_build_count(), 3);
    assert_ne!(after_disconnect, after_readiness);
}

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
        audit: nodelite_proto::AuditConfig {
            enabled: true,
            db_path: PathBuf::from("/tmp/nodelite-test-audit.sqlite3"),
            retention_days: 90,
            log_successful_auth: true,
            log_failed_auth: true,
            log_token_events: true,
            log_rate_limit: true,
        },
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
