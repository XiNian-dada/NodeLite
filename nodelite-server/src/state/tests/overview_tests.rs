use super::*;

#[test]
fn overview_saturates_totals_and_skips_invalid_rates() {
    let mut registry = Registry::default();
    let now = Utc
        .with_ymd_and_hms(2026, 5, 7, 0, 0, 0)
        .single()
        .expect("valid test datetime");

    registry.register_node(
        1,
        sample_identity(),
        Some("198.51.100.10".to_string()),
        None,
        now,
    );
    registry.register_node(
        2,
        NodeIdentity {
            node_id: "sg-01".to_string(),
            node_label: "Singapore 01".to_string(),
            ..sample_identity()
        },
        Some("198.51.100.11".to_string()),
        None,
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
    let mut registry = Registry::default();
    let now = Utc
        .with_ymd_and_hms(2026, 5, 7, 0, 0, 0)
        .single()
        .expect("valid test datetime");

    registry.register_node(
        1,
        sample_identity(),
        Some("198.51.100.10".to_string()),
        None,
        now,
    );
    registry.register_node(
        2,
        NodeIdentity {
            node_id: "sg-01".to_string(),
            node_label: "Singapore 01".to_string(),
            ..sample_identity()
        },
        Some("198.51.100.11".to_string()),
        None,
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
