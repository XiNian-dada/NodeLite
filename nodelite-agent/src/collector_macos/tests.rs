use std::time::{Duration, Instant};

use super::super::shared::{NetworkSample, NetworkTotals, compute_network_rates};
use super::identity::{extract_plist_value, parse_os_name_from_plist};
use super::metrics::{
    NetworkInterfaceCache, NetworkInterfaceSignature, ObservedNetworkSample,
    compute_available_memory_bytes, compute_network_rates_if_same_interfaces,
};

#[test]
fn extracts_plist_string_value() {
    let content = r#"
        <plist version="1.0">
          <dict>
            <key>ProductName</key>
            <string>macOS</string>
            <key>ProductVersion</key>
            <string>15.5</string>
          </dict>
        </plist>
    "#;
    assert_eq!(
        extract_plist_value(content, "ProductName").as_deref(),
        Some("macOS")
    );
    assert_eq!(
        extract_plist_value(content, "ProductVersion").as_deref(),
        Some("15.5")
    );
}

#[test]
fn parses_os_name_from_system_version_plist() {
    let content = r#"
        <plist version="1.0">
          <dict>
            <key>ProductName</key>
            <string>macOS</string>
            <key>ProductVersion</key>
            <string>15.5</string>
          </dict>
        </plist>
    "#;

    assert_eq!(
        parse_os_name_from_plist(content).expect("plist should parse"),
        "macOS 15.5"
    );
}

#[test]
fn rejects_plist_without_product_fields() {
    let content = r#"
        <plist version="1.0">
          <dict>
            <key>BuildVersion</key>
            <string>24F74</string>
          </dict>
        </plist>
    "#;

    let error = parse_os_name_from_plist(content).expect_err("plist should be rejected");
    assert!(
        error
            .to_string()
            .contains("ProductName/ProductVersion missing from SystemVersion.plist")
    );
}

#[test]
fn computes_network_rates_from_deltas() {
    let previous = NetworkSample {
        observed_at: Instant::now() - Duration::from_secs(2),
        rx_bytes: 100,
        tx_bytes: 40,
    };
    let current = NetworkTotals {
        rx_bytes: 220,
        tx_bytes: 100,
    };
    let (rx_rate, tx_rate) = compute_network_rates(previous, Instant::now(), current);
    assert!(rx_rate.expect("rx rate should be present") > 50.0);
    assert!(tx_rate.expect("tx rate should be present") > 20.0);
}

#[test]
fn skips_network_rates_when_interface_signature_changes() {
    let previous = ObservedNetworkSample {
        sample: NetworkSample {
            observed_at: Instant::now() - Duration::from_secs(2),
            rx_bytes: 100,
            tx_bytes: 40,
        },
        signature: NetworkInterfaceSignature::from_indices(&[4]),
    };
    let current = NetworkTotals {
        rx_bytes: 10_000_000_000,
        tx_bytes: 4_000_000_000,
    };

    let (rx_rate, tx_rate) = compute_network_rates_if_same_interfaces(
        &previous,
        Instant::now(),
        current,
        &NetworkInterfaceSignature::from_indices(&[4, 5]),
    );

    assert_eq!(rx_rate, None);
    assert_eq!(tx_rate, None);
}

#[test]
fn network_interface_signature_order_is_stable() {
    assert_eq!(
        NetworkInterfaceSignature::from_indices(&[5, 4]),
        NetworkInterfaceSignature::from_indices(&[4, 5]),
    );
    assert_ne!(
        NetworkInterfaceSignature::from_indices(&[4]),
        NetworkInterfaceSignature::from_indices(&[4, 5]),
    );
}

#[test]
fn network_interface_cache_only_matches_same_non_empty_list() {
    let mut cache = NetworkInterfaceCache {
        list_len: Some(4096),
        indices: vec![4, 5],
    };

    assert!(cache.can_sample_cached_indices(4096));
    assert!(!cache.can_sample_cached_indices(4097));

    cache.clear();
    assert!(!cache.can_sample_cached_indices(4096));
    assert!(cache.indices.is_empty());
}

#[test]
fn available_memory_does_not_underflow_when_compressor_is_large() {
    let mut stats = unsafe { std::mem::zeroed::<libc::vm_statistics64>() };
    stats.free_count = 5_431;
    stats.inactive_count = 520_105;
    stats.purgeable_count = 18_475;
    stats.compressor_page_count = 786_007;

    let available = compute_available_memory_bytes(&stats, 16_384, 34_359_738_368);
    assert!(available > 0);
    assert!(available < 34_359_738_368);
}
