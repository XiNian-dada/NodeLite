//! Independent Metrics frames avoid compressing credentials or sharing compression dictionaries.

use std::io::Write;

use flate2::{Compression, Decompress, FlushDecompress, Status, write::ZlibEncoder};
use thiserror::Error;

use crate::MetricsMessage;

const MAGIC: &[u8] = b"NLMZ1";
pub const MAX_METRICS_JSON_BYTES: usize = 1024 * 1024;

#[derive(Debug, Error)]
pub enum CompressionError {
    #[error("compressed metrics exceed the decoded message limit")]
    TooLarge,
    #[error("invalid or incomplete compressed metrics frame")]
    InvalidFrame,
    #[error("encode metrics compression: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid metrics JSON: {0}")]
    Json(#[from] serde_json::Error),
}

/// Includes a format marker; each frame can be decoded without any preceding frame.
pub fn encode_metrics(message: &MetricsMessage) -> Result<Vec<u8>, CompressionError> {
    let json = serde_json::to_vec(message)?;
    if json.len() > MAX_METRICS_JSON_BYTES {
        return Err(CompressionError::TooLarge);
    }
    let mut encoder = ZlibEncoder::new(MAGIC.to_vec(), Compression::default());
    encoder.write_all(&json)?;
    Ok(encoder.finish()?)
}

/// The configured plaintext limit still applies, even to a tiny compressed input.
pub fn decode_metrics(frame: &[u8], limit: usize) -> Result<MetricsMessage, CompressionError> {
    let input = frame
        .strip_prefix(MAGIC)
        .ok_or(CompressionError::InvalidFrame)?;
    let limit = limit.min(MAX_METRICS_JSON_BYTES);
    if frame.len() > limit {
        return Err(CompressionError::TooLarge);
    }
    let mut decoder = Decompress::new(true);
    let mut output = Vec::new();
    let mut chunk = [0_u8; 4096];
    loop {
        let before = (decoder.total_in(), decoder.total_out());
        let capacity = chunk.len().min(limit.saturating_sub(output.len()) + 1);
        let status = decoder
            .decompress(
                &input[before.0 as usize..],
                &mut chunk[..capacity],
                FlushDecompress::None,
            )
            .map_err(|_| CompressionError::InvalidFrame)?;
        let written = (decoder.total_out() - before.1) as usize;
        if output.len() + written > limit {
            return Err(CompressionError::TooLarge);
        }
        output.extend_from_slice(&chunk[..written]);
        if status == Status::StreamEnd {
            if decoder.total_in() as usize != input.len() {
                return Err(CompressionError::InvalidFrame);
            }
            return Ok(serde_json::from_slice(&output)?);
        }
        if before == (decoder.total_in(), decoder.total_out()) {
            return Err(CompressionError::InvalidFrame);
        }
    }
}

#[cfg(test)]
mod tests;
