use std::mem;
use std::ptr;
use std::time::{Duration, Instant};

use super::super::shared::{NetworkSample, NetworkTotals, compute_network_metrics};
use super::identity::{extract_plist_value, parse_os_name_from_plist};
use super::metrics::{
    NetworkInterfaceCache, NetworkInterfaceSignature, ObservedNetworkSample,
    compute_available_memory_bytes, compute_network_metrics_if_same_interfaces,
    parse_network_totals_and_indices_from_iflist2,
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
        rx_packets: 10,
        tx_packets: 4,
        rx_dropped_packets: 0,
        tx_dropped_packets: 0,
    };
    let current = NetworkTotals {
        rx_bytes: 220,
        tx_bytes: 100,
        rx_packets: 22,
        tx_packets: 10,
        rx_dropped_packets: 0,
        tx_dropped_packets: 0,
    };
    let metrics = compute_network_metrics(previous, Instant::now(), current);
    assert!(metrics.rx_bytes_per_sec.expect("rx rate should be present") > 50.0);
    assert!(metrics.tx_bytes_per_sec.expect("tx rate should be present") > 20.0);
}

#[test]
fn skips_network_metrics_when_interface_signature_changes() {
    let previous = ObservedNetworkSample {
        sample: NetworkSample {
            observed_at: Instant::now() - Duration::from_secs(2),
            rx_bytes: 100,
            tx_bytes: 40,
            rx_packets: 10,
            tx_packets: 4,
            rx_dropped_packets: 0,
            tx_dropped_packets: 0,
        },
        signature: NetworkInterfaceSignature::from_indices(&[4]),
    };
    let current = NetworkTotals {
        rx_bytes: 10_000_000_000,
        tx_bytes: 4_000_000_000,
        rx_packets: 10_000,
        tx_packets: 4_000,
        rx_dropped_packets: 10,
        tx_dropped_packets: 0,
    };

    let metrics = compute_network_metrics_if_same_interfaces(
        &previous,
        Instant::now(),
        current,
        &NetworkInterfaceSignature::from_indices(&[4, 5]),
    );

    assert_eq!(metrics.rx_bytes_per_sec, None);
    assert_eq!(metrics.tx_bytes_per_sec, None);
    assert_eq!(metrics.packet_loss_percent, None);
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
fn iflist2_parser_ignores_truncated_headers() {
    let buffer = vec![0_u8; mem::size_of::<libc::if_msghdr>().saturating_sub(1)];
    let (totals, indices) = parse_network_totals_and_indices_from_iflist2(&buffer);

    assert_eq!(totals.rx_bytes, 0);
    assert_eq!(totals.tx_bytes, 0);
    assert!(indices.is_empty());
}

#[test]
fn iflist2_parser_stops_on_overlong_messages() {
    let mut buffer = vec![0_u8; mem::size_of::<libc::if_msghdr>()];
    // SAFETY: `if_msghdr` is a plain C header used here only as a byte fixture;
    // zeroed fields are overwritten as needed before the unaligned write.
    let mut header = unsafe { mem::zeroed::<libc::if_msghdr>() };
    header.ifm_msglen = (buffer.len() + 1) as _;
    header.ifm_type = libc::RTM_IFINFO2 as _;
    // SAFETY: `buffer` has exactly enough bytes for one header, and
    // `write_unaligned` matches the parser's byte-stream assumptions.
    unsafe {
        ptr::write_unaligned(buffer.as_mut_ptr().cast::<libc::if_msghdr>(), header);
    }

    let (totals, indices) = parse_network_totals_and_indices_from_iflist2(&buffer);

    assert_eq!(totals.rx_packets, 0);
    assert_eq!(totals.tx_packets, 0);
    assert!(indices.is_empty());
}

#[test]
fn available_memory_does_not_underflow_when_compressor_is_large() {
    // SAFETY: `vm_statistics64` is a plain C statistics struct; this test
    // assigns the fields used by `compute_available_memory_bytes`.
    let mut stats = unsafe { std::mem::zeroed::<libc::vm_statistics64>() };
    stats.free_count = 5_431;
    stats.inactive_count = 520_105;
    stats.purgeable_count = 18_475;
    stats.compressor_page_count = 786_007;

    let available = compute_available_memory_bytes(&stats, 16_384, 34_359_738_368);
    assert!(available > 0);
    assert!(available < 34_359_738_368);
}
