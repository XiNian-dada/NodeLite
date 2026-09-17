//! Compression must preserve snapshots while rejecting oversized, truncated, or mixed frames.

use super::*;

fn message() -> MetricsMessage {
    serde_json::from_value(serde_json::json!({"snapshot": {
        "collected_at": "2026-09-14T00:00:00Z", "cpu_usage_percent": 12.5,
        "load": {"one": 0.1, "five": 0.2, "fifteen": 0.3},
        "memory": {"total_bytes": 1024, "used_bytes": 512, "available_bytes": 512, "swap_total_bytes": 0, "swap_used_bytes": 0},
        "uptime_secs": 42, "disks": [],
        "network": {"total_rx_bytes": 123, "total_tx_bytes": 456, "rx_bytes_per_sec": null, "tx_bytes_per_sec": null, "packet_loss_percent": null}
    }})).expect("sample metrics")
}

#[test]
fn metrics_roundtrip_and_exact_decoded_limit() {
    let message = message();
    let bytes = serde_json::to_vec(&message).expect("JSON").len();
    let encoded = encode_metrics(&message).expect("encode");
    assert_eq!(decode_metrics(&encoded, bytes).expect("decode"), message);
    assert!(matches!(
        decode_metrics(&encoded, bytes - 1),
        Err(CompressionError::TooLarge)
    ));
}

#[test]
fn rejects_truncation_corruption_trailing_data_and_unframed_input() {
    let encoded = encode_metrics(&message()).expect("encode");
    for length in 0..encoded.len() {
        assert!(
            decode_metrics(&encoded[..length], 65536).is_err(),
            "length={length}"
        );
    }
    let mut corrupt = encoded.clone();
    let last = corrupt.len() - 1;
    corrupt[last] ^= 1;
    assert!(decode_metrics(&corrupt, 65536).is_err());
    let mut trailing = encoded.clone();
    trailing.extend_from_slice(&encoded);
    assert!(decode_metrics(&trailing, 65536).is_err());
    assert!(decode_metrics(br#"{"snapshot": {}}"#, 65536).is_err());
}

#[test]
fn rejects_decompression_bombs_and_non_metrics_json() {
    for json in [
        vec![b' '; MAX_METRICS_JSON_BYTES + 1],
        br#"{"type":"hello","token":"test"}"#.to_vec(),
    ] {
        let mut encoder = ZlibEncoder::new(MAGIC.to_vec(), Compression::default());
        encoder.write_all(&json).expect("compress test input");
        let frame = encoder.finish().expect("finish");
        assert!(decode_metrics(&frame, usize::MAX).is_err());
    }
}
