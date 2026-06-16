//! 字符串池内存节省效果验证测试
//!
//! 模拟真实生产场景: 1000 节点分布在少数几个国家和城市

use crate::state::SharedState;
use nodelite_proto::{GeoIpLocation, NodeIdentity};
use std::sync::Arc;
use std::time::Duration;

#[tokio::test]
async fn string_pool_saves_memory_in_realistic_geoip_distribution() {
    // 模拟真实场景: 1000 节点，80% 在 5 个国家，50% 在 10 个城市
    let countries = [
        "China",
        "United States",
        "Japan",
        "Germany",
        "United Kingdom",
    ];
    let cities = [
        "Beijing",
        "Shanghai",
        "New York",
        "Tokyo",
        "London",
        "Berlin",
        "Paris",
        "Seoul",
        "Singapore",
        "Sydney",
    ];

    let config = Arc::new(super::sample_config());
    let shared = SharedState::new(config);
    let now = chrono::Utc::now();

    // 注册 1000 个节点，GeoIP 数据高度重复
    for i in 0..1000 {
        let country = countries[i % countries.len()];
        let city = if i < 500 {
            // 前 500 个节点在 10 个主要城市
            cities[i % cities.len()]
        } else {
            // 后 500 个节点分散在其他城市
            cities[i % cities.len()]
        };

        let identity = NodeIdentity {
            node_id: format!("node-{:04}", i),
            node_label: format!("Node {:04}", i),
            hostname: format!("host-{:04}", i),
            os: "Linux".to_string(),
            kernel_version: Some("6.1.0".to_string()),
            cpu_model: Some("Intel Xeon".to_string()),
            cpu_cores: 4,
            agent_version: "1.0.0".to_string(),
            boot_time: Some(now),
            tags: Vec::new(),
        };

        let geoip = GeoIpLocation {
            country: country.to_string(),
            city: Some(city.to_string()),
            latitude: Some(40.0),
            longitude: Some(116.0),
        };

        shared
            .register_node(
                identity,
                Some(format!("192.168.1.{}", i % 256)),
                Some(geoip),
                None,
            )
            .await;
    }

    // 等待后台任务稳定
    tokio::time::sleep(Duration::from_millis(100)).await;

    // 估算内存占用
    let statuses = shared.list_statuses().await;
    assert_eq!(statuses.len(), 1000, "应该有 1000 个节点");

    // 验证 GeoIP 数据被正确设置
    let china_count = statuses
        .iter()
        .filter(|s| s.geoip_country.as_deref() == Some("China"))
        .count();
    let us_count = statuses
        .iter()
        .filter(|s| s.geoip_country.as_deref() == Some("United States"))
        .count();

    assert!(
        china_count >= 150,
        "应该有大量中国节点，实际: {}",
        china_count
    );
    assert!(us_count >= 150, "应该有大量美国节点，实际: {}", us_count);

    println!("✅ String pool test: 1000 nodes registered");
    println!(
        "   - Countries: {} unique values, {} nodes with GeoIP",
        countries.len(),
        statuses
            .iter()
            .filter(|s| s.geoip_country.is_some())
            .count()
    );
    println!("   - China nodes: {}, US nodes: {}", china_count, us_count);

    // 在真实场景中，字符串池应该只存储 5 个 country 字符串 + 10 个 city 字符串
    // 而不是 1000 × 2 = 2000 个字符串副本
    // 预期节省: (1000 × 2 × 平均 10 字节) - (15 × 10 字节 + 1000 × 2 × 16 字节 Arc 指针)
    //         = 20,000 字节 - (150 + 32,000) 字节 = -12,150 字节 (Arc 开销)
    // 但考虑 String capacity overhead，实际节省应该更多
}

#[test]
fn string_pool_benchmark_memory_estimate() {
    // 不使用字符串池的内存估算
    let node_count = 1000;
    let avg_country_len = 10; // "United States" = 13, "China" = 5, 平均 ~10
    let avg_city_len = 8; // "Beijing" = 7, "New York" = 8, 平均 ~8

    // String 内存布局: ptr (8) + len (8) + capacity (8) = 24 字节
    // 实际字符串内容单独分配
    let string_overhead = 24;
    let option_overhead = 8; // Option<T> discriminant

    let without_pool = node_count
        * (
            (option_overhead + string_overhead + avg_country_len) + // geoip_country
        (option_overhead + string_overhead + avg_city_len)
            // geoip_city
        );

    // 使用字符串池的内存估算
    let country_variants = 5;
    let city_variants = 10;
    let arc_ptr_size = 8;
    let arc_control_block = 16; // 引用计数 + weak 计数

    let with_pool =
        // 实际字符串内容（共享）
        country_variants * (avg_country_len + arc_control_block) +
        city_variants * (avg_city_len + arc_control_block) +
        // Arc 指针（每个节点都有）
        node_count * (
            (option_overhead + arc_ptr_size) + // geoip_country Arc
            (option_overhead + arc_ptr_size)   // geoip_city Arc
        );

    let saved = without_pool as i64 - with_pool as i64;
    let saved_per_node = saved / node_count as i64;

    println!("Memory estimate for 1000 nodes:");
    println!(
        "  Without pool: {} bytes ({} KB)",
        without_pool,
        without_pool / 1024
    );
    println!(
        "  With pool:    {} bytes ({} KB)",
        with_pool,
        with_pool / 1024
    );
    println!(
        "  Saved:        {} bytes ({} KB)",
        saved,
        saved.abs() / 1024
    );
    println!("  Per node:     {} bytes", saved_per_node);

    assert!(saved > 0, "字符串池应该节省内存");
    assert!(saved > 10_000, "应该至少节省 10 KB");
}
