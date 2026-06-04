use super::*;

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
