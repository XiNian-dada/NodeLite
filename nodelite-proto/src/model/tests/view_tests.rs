//! 测试 Phase 3.2 NodeListItemView 的序列化正确性和 Arc 共享行为。

use std::sync::Arc;

use crate::{
    NodeListIdentity, NodeListItem, NodeListItemView, NodeListLoadAverage, NodeListMemoryUsage,
    NodeListSnapshot,
};

fn sample_identity() -> NodeListIdentity {
    NodeListIdentity {
        node_id: "test-node-1".to_string(),
        node_label: "Test Node".to_string(),
        hostname: "test.example.com".to_string(),
        tags: vec!["production".to_string(), "us-west".to_string()],
    }
}

fn sample_snapshot() -> NodeListSnapshot {
    NodeListSnapshot {
        cpu_usage_percent: Some(42.5),
        load: NodeListLoadAverage { one: 1.23 },
        memory: NodeListMemoryUsage {
            total_bytes: 16_000_000_000,
            used_bytes: 8_000_000_000,
        },
    }
}

#[test]
fn node_list_item_view_serializes_identically_to_node_list_item() {
    let view = NodeListItemView {
        identity: sample_identity(),
        geoip_country: Some(Arc::from("US")),
        geoip_city: Some(Arc::from("San Francisco")),
        geoip_latitude: Some(37.7749),
        geoip_longitude: Some(-122.4194),
        location_override_country: Some(Arc::from("CN")),
        location_override_city: Some(Arc::from("Beijing")),
        location_override_latitude: Some(39.9042),
        location_override_longitude: Some(116.4074),
        snapshot: Some(sample_snapshot()),
        latency_ms: Some(42),
        online: true,
    };

    let item = NodeListItem {
        identity: sample_identity(),
        geoip_country: Some("US".to_string()),
        geoip_city: Some("San Francisco".to_string()),
        geoip_latitude: Some(37.7749),
        geoip_longitude: Some(-122.4194),
        location_override_country: Some("CN".to_string()),
        location_override_city: Some("Beijing".to_string()),
        location_override_latitude: Some(39.9042),
        location_override_longitude: Some(116.4074),
        snapshot: Some(sample_snapshot()),
        latency_ms: Some(42),
        online: true,
    };

    let view_json = serde_json::to_value(&view).expect("view should serialize");
    let item_json = serde_json::to_value(&item).expect("item should serialize");

    assert_eq!(view_json, item_json, "JSON output should be identical");
}

#[test]
fn node_list_item_view_shares_arc_references() {
    let country = Arc::from("US");
    let country_clone = Arc::clone(&country);

    let view = NodeListItemView {
        identity: sample_identity(),
        geoip_country: Some(country),
        geoip_city: None,
        geoip_latitude: None,
        geoip_longitude: None,
        location_override_country: None,
        location_override_city: None,
        location_override_latitude: None,
        location_override_longitude: None,
        snapshot: None,
        latency_ms: None,
        online: false,
    };

    // 验证 Arc 被共享 (引用计数 >= 2: country_clone + view.geoip_country)
    assert!(
        Arc::strong_count(&country_clone) >= 2,
        "Arc should be shared between clone and view"
    );

    // 序列化后引用计数应该不变 (序列化只读取内容,不克隆 Arc)
    let count_before = Arc::strong_count(&country_clone);
    let _json = serde_json::to_string(&view).expect("should serialize");
    let count_after = Arc::strong_count(&country_clone);

    assert_eq!(
        count_before, count_after,
        "Serialization should not increase Arc refcount"
    );
}

#[test]
fn node_list_item_view_serializes_none_as_null() {
    let view = NodeListItemView {
        identity: sample_identity(),
        geoip_country: None,
        geoip_city: Some(Arc::from("Tokyo")),
        geoip_latitude: None,
        geoip_longitude: None,
        location_override_country: None,
        location_override_city: None,
        location_override_latitude: None,
        location_override_longitude: None,
        snapshot: Some(sample_snapshot()),
        latency_ms: Some(10),
        online: true,
    };

    let item = NodeListItem {
        identity: sample_identity(),
        geoip_country: None,
        geoip_city: Some("Tokyo".to_string()),
        geoip_latitude: None,
        geoip_longitude: None,
        location_override_country: None,
        location_override_city: None,
        location_override_latitude: None,
        location_override_longitude: None,
        snapshot: Some(sample_snapshot()),
        latency_ms: Some(10),
        online: true,
    };

    let view_json = serde_json::to_value(&view).expect("view should serialize");
    let item_json = serde_json::to_value(&item).expect("item should serialize");

    // None 字段应该序列化为 null,与 NodeListItem 一致
    assert_eq!(
        view_json, item_json,
        "None fields should serialize as null, matching NodeListItem"
    );
    assert_eq!(view_json["geoip_country"], serde_json::Value::Null);
    assert_eq!(view_json["geoip_city"], "Tokyo");
}

#[test]
fn node_list_item_view_clone_is_cheap() {
    let view = NodeListItemView {
        identity: sample_identity(),
        geoip_country: Some(Arc::from("US")),
        geoip_city: Some(Arc::from("New York")),
        geoip_latitude: Some(40.7128),
        geoip_longitude: Some(-74.0060),
        location_override_country: None,
        location_override_city: None,
        location_override_latitude: None,
        location_override_longitude: None,
        snapshot: Some(sample_snapshot()),
        latency_ms: Some(5),
        online: true,
    };

    let country_ref = view.geoip_country.as_ref().unwrap();
    let initial_count = Arc::strong_count(country_ref);

    // Clone NodeListItemView (应该只增加 Arc 引用计数,不复制字符串)
    let _cloned = view.clone();

    let final_count = Arc::strong_count(country_ref);

    // 引用计数应该增加 1 (原 view + cloned view)
    assert_eq!(
        final_count,
        initial_count + 1,
        "Clone should only increment Arc refcount, not copy strings"
    );
}
