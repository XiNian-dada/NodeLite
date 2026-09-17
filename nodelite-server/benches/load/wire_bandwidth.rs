//! Count the exact payload and masked WebSocket framing bytes for changing real Metrics messages.

use std::time::Instant;

use anyhow::{Result, ensure};
use nodelite_proto::{
    MetricsMessage, WireMessage,
    compression::{decode_metrics, encode_metrics},
};

use super::fake_agent::fake_snapshot_with_disks;

pub(super) fn run() -> Result<()> {
    for disks in [1, 8, 16, 64] {
        let mut text_bytes = 0;
        let mut compressed_bytes = 0;
        let mut encode_micros = 0;
        for sample in 1..=1000 {
            let message = MetricsMessage {
                snapshot: fake_snapshot_with_disks(sample, disks),
            };
            let text = serde_json::to_vec(&WireMessage::Metrics(message.clone()))?;
            let started = Instant::now();
            let compressed = encode_metrics(&message)?;
            encode_micros += started.elapsed().as_micros();
            text_bytes += frame_bytes(text.len());
            compressed_bytes += frame_bytes(compressed.len());
            ensure!(
                decode_metrics(&compressed, 65536)? == message,
                "compressed snapshot differs"
            );
        }
        let saved = 100.0 * (1.0 - compressed_bytes as f64 / text_bytes as f64);
        println!(
            "WIRE_BANDWIDTH_RESULT disks={disks} samples=1000 text_frame_bytes={text_bytes} compressed_frame_bytes={compressed_bytes} saved_percent={saved:.2} encode_mean_us={:.2}",
            encode_micros as f64 / 1000.0
        );
        // The issue's 60% target describes ~2 KiB snapshots; retain the small-frame
        // result too because independent compression has a fixed dictionary cost.
        let target = if disks == 1 { 0.0 } else { 60.0 };
        ensure!(
            saved > target,
            "Metrics compression missed the workload bandwidth target"
        );
    }
    Ok(())
}

fn frame_bytes(payload: usize) -> usize {
    payload
        + 6
        + match payload {
            0..=125 => 0,
            126..=65535 => 2,
            _ => 8,
        }
}
