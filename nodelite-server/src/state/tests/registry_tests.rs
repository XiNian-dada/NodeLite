use super::*;
use nodelite_proto::NodeStatus;

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

    registry.register_node(
        1,
        identity.clone(),
        Some("198.51.100.10".to_string()),
        None,
        now,
    );
    registry.register_node(
        2,
        identity,
        Some("198.51.100.11".to_string()),
        None,
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
fn newer_session_refreshes_remote_ip_and_geoip() {
    let mut registry = Registry::default();
    let now = Utc
        .with_ymd_and_hms(2026, 5, 7, 0, 0, 0)
        .single()
        .expect("valid test datetime");
    let identity = sample_identity();

    registry.register_node(
        1,
        identity.clone(),
        Some("198.51.100.10".to_string()),
        Some(GeoIpLocation {
            country: "US".to_string(),
            city: Some("Mountain View".to_string()),
            latitude: Some(37.386),
            longitude: Some(-122.0838),
        }),
        now,
    );
    registry.register_node(
        2,
        identity,
        Some("203.0.113.20".to_string()),
        Some(GeoIpLocation {
            country: "JP".to_string(),
            city: Some("Tokyo".to_string()),
            latitude: Some(35.6762),
            longitude: Some(139.6503),
        }),
        now + ChronoDuration::seconds(3),
    );

    let status = registry
        .list_statuses()
        .into_iter()
        .find(|node| node.identity.node_id == "hk-01")
        .expect("node status");
    assert_eq!(status.remote_ip.as_deref(), Some("203.0.113.20"));
    assert_eq!(status.geoip_country.as_deref(), Some("JP"));
    assert_eq!(status.geoip_city.as_deref(), Some("Tokyo"));

    let summary = registry
        .list_node_summaries()
        .into_iter()
        .find(|node| node.identity.node_id == "hk-01")
        .expect("node summary");
    assert_eq!(summary.geoip_country.as_deref(), Some("JP"));
    assert_eq!(summary.geoip_city.as_deref(), Some("Tokyo"));
}

#[test]
fn stale_nodes_are_marked_offline() {
    let mut registry = Registry::default();
    let now = Utc
        .with_ymd_and_hms(2026, 5, 7, 0, 0, 0)
        .single()
        .expect("valid test datetime");

    registry.register_node(
        7,
        sample_identity(),
        Some("198.51.100.10".to_string()),
        None,
        now,
    );
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
fn session_control_is_only_available_for_current_online_session() {
    let mut registry = Registry::default();
    let now = Utc
        .with_ymd_and_hms(2026, 5, 7, 0, 0, 0)
        .single()
        .expect("valid test datetime");
    registry.register_node(
        7,
        sample_identity(),
        Some("198.51.100.10".to_string()),
        None,
        now,
    );

    let (control, _control_rx) = SessionControlHandle::channel();
    assert!(registry.attach_session_control("hk-01", 7, control));
    assert!(registry.session_control("hk-01").is_some());

    registry.register_node(
        8,
        sample_identity(),
        Some("198.51.100.11".to_string()),
        None,
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
    registry.register_node(
        9,
        sample_identity(),
        Some("198.51.100.10".to_string()),
        None,
        now,
    );

    let (control, _control_rx) = SessionControlHandle::channel();
    assert!(registry.attach_session_control("hk-01", 9, control));
    registry.mark_disconnected("hk-01", 9);

    assert!(registry.session_control("hk-01").is_none());
}

#[test]
fn runtime_entry_is_smaller_than_cached_external_models() {
    assert!(
        Registry::runtime_entry_inline_bytes_for_test()
            < Registry::previous_external_model_inline_bytes_for_test(),
        "registry entries should not inline-cache NodeStatus plus NodeListItem",
    );
}

#[test]
fn runtime_entry_retained_heap_is_lower_than_cached_external_models() {
    let mut identity = sample_identity();
    identity.node_id = "hong-kong-edge-node-0001".repeat(2);
    identity.node_label = "Hong Kong Edge Node With Long Display Label".repeat(2);
    identity.hostname = "hk-edge-node-0001.example.internal".repeat(2);
    identity.os = "linux-production".repeat(2);
    identity.kernel_version = Some("6.8.0-nodelite-production".repeat(2));
    identity.cpu_model = Some("Example Compute Processor 9000".repeat(2));
    identity.agent_version = "0.1.0-review-build".repeat(2);
    identity.tags = (0..16)
        .map(|index| format!("fleet:edge-region-hk-{index:02}"))
        .collect();

    let mut snapshot = sample_snapshot(Utc::now());
    snapshot.disks = (0..128)
        .map(|index| DiskUsage {
            device: format!("/dev/disk/by-id/nodelite-review-device-{index:03}"),
            mount_point: format!("/srv/nodelite/review/mount/{index:03}"),
            fs_type: "ext4".to_string(),
            total_bytes: 1024 * 1024 * 1024,
            available_bytes: 512 * 1024 * 1024,
            used_bytes: 512 * 1024 * 1024,
            used_percent: 50.0,
        })
        .collect();
    let status = NodeStatus {
        identity,
        remote_ip: Some("198.51.100.10".to_string()),
        geoip_country: Some("HK".to_string()),
        geoip_city: Some("Hong Kong".to_string()),
        geoip_latitude: Some(22.3193),
        geoip_longitude: Some(114.1694),
        snapshot: Some(snapshot),
        last_seen: Some(Utc::now()),
        latency_ms: Some(42),
        online: true,
    };

    let (runtime, previous) = Registry::retained_heap_estimates_for_test(status);

    assert!(
        runtime.bytes < previous.bytes,
        "runtime entry retained heap bytes should be lower than cached external models: runtime={}, previous={}",
        runtime.bytes,
        previous.bytes,
    );
    assert!(
        runtime.allocations < previous.allocations,
        "runtime entry heap buffer count should be lower than cached external models: runtime={}, previous={}",
        runtime.allocations,
        previous.allocations,
    );
}

#[tokio::test]
async fn restore_statuses_reassembles_detail_and_lightweight_api_views() {
    let shared = SharedState::new(Arc::new(sample_config()));
    let mut snapshot = sample_snapshot(Utc::now());
    snapshot.disks.resize_with(4, sample_disk_usage);
    let restored = NodeStatus {
        identity: sample_identity(),
        remote_ip: Some("198.51.100.10".to_string()),
        geoip_country: Some("HK".to_string()),
        geoip_city: Some("Hong Kong".to_string()),
        geoip_latitude: Some(22.3193),
        geoip_longitude: Some(114.1694),
        snapshot: Some(snapshot),
        last_seen: Some(Utc::now()),
        latency_ms: Some(42),
        online: true,
    };

    shared.restore_statuses(vec![restored]).await;

    let detail = shared.get_status("hk-01").await.expect("restored status");
    assert!(
        !detail.online,
        "restored nodes stay offline until a new session connects"
    );
    assert_eq!(detail.remote_ip.as_deref(), Some("198.51.100.10"));
    assert_eq!(
        detail
            .snapshot
            .as_ref()
            .map(|snapshot| snapshot.disks.len()),
        Some(4),
    );

    let nodes_body = shared
        .nodes_json_bytes()
        .await
        .expect("nodes api body should serialize");
    let nodes_body = std::str::from_utf8(&nodes_body).expect("nodes body should be utf-8");
    assert!(nodes_body.contains("\"node_id\":\"hk-01\""));
    assert!(
        !nodes_body.contains("\"disks\""),
        "list API should assemble NodeListItem instead of serializing full snapshots",
    );
}

#[tokio::test]
async fn registry_disk_entries_total_counts_snapshot_disks() {
    let shared = SharedState::new(Arc::new(sample_config()));
    let first_session = shared
        .register_node(sample_identity(), Some("198.51.100.10".to_string()), None)
        .await;
    let second_session = shared
        .register_node(
            NodeIdentity {
                node_id: "sg-01".to_string(),
                node_label: "Singapore 01".to_string(),
                ..sample_identity()
            },
            Some("198.51.100.11".to_string()),
            None,
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
async fn refresh_geoip_locations_updates_online_node_view() {
    let shared = SharedState::new(Arc::new(sample_config()));
    let _session_id = shared
        .register_node(sample_identity(), Some("8.8.8.8".to_string()), None)
        .await;

    assert_eq!(
        shared.geoip_refresh_candidates().await,
        vec![("hk-01".to_string(), "8.8.8.8".to_string())]
    );

    let updated = shared
        .refresh_geoip_locations(vec![(
            "hk-01".to_string(),
            "8.8.8.8".to_string(),
            GeoIpLocation {
                country: "US".to_string(),
                city: Some("Mountain View".to_string()),
                latitude: Some(37.386),
                longitude: Some(-122.0838),
            },
        )])
        .await;

    assert_eq!(updated, 1);
    let status = shared.get_status("hk-01").await.expect("node status");
    assert_eq!(status.geoip_country.as_deref(), Some("US"));
    assert_eq!(status.geoip_city.as_deref(), Some("Mountain View"));
    assert_eq!(status.geoip_latitude, Some(37.386));
    assert_eq!(status.geoip_longitude, Some(-122.0838));

    let summary = shared
        .list_node_summaries()
        .await
        .into_iter()
        .find(|node| node.identity.node_id == "hk-01")
        .expect("node summary");
    assert_eq!(summary.geoip_country.as_deref(), Some("US"));
    assert_eq!(summary.geoip_city.as_deref(), Some("Mountain View"));
}

#[tokio::test]
async fn refresh_geoip_locations_skips_stale_remote_ip() {
    let shared = SharedState::new(Arc::new(sample_config()));
    shared
        .register_node(sample_identity(), Some("8.8.8.8".to_string()), None)
        .await;

    let updated = shared
        .refresh_geoip_locations(vec![(
            "hk-01".to_string(),
            "1.1.1.1".to_string(),
            GeoIpLocation {
                country: "US".to_string(),
                city: None,
                latitude: None,
                longitude: None,
            },
        )])
        .await;

    assert_eq!(updated, 0);
    let status = shared.get_status("hk-01").await.expect("node status");
    assert_eq!(status.geoip_country, None);
}
