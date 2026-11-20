//! Saturating time arithmetic for config-driven alert durations.
//!
//! Config parsing already bounds these values; saturating here keeps a value that bypassed
//! parsing (e.g. a future settings path) from panicking the alert task.

use jiff::SignedDuration;
use nodelite_proto::timestamp::UtcTimestamp;

pub(crate) fn minutes(value: u64) -> SignedDuration {
    i64::try_from(value)
        .ok()
        .map(|value| SignedDuration::from_secs(value).saturating_mul(60))
        .unwrap_or(SignedDuration::MAX)
}

pub(crate) fn hours(value: u64) -> SignedDuration {
    i64::try_from(value)
        .ok()
        .map(|value| SignedDuration::from_secs(value).saturating_mul(3600))
        .unwrap_or(SignedDuration::MAX)
}

pub(crate) fn sub(at: UtcTimestamp, delta: SignedDuration) -> UtcTimestamp {
    at.saturating_sub(delta)
}

pub(crate) fn add(at: UtcTimestamp, delta: SignedDuration) -> UtcTimestamp {
    at.saturating_add(delta)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oversized_values_saturate_instead_of_panicking() {
        let now = UtcTimestamp::now();
        assert_eq!(minutes(u64::MAX), SignedDuration::MAX);
        assert_eq!(minutes(i64::MAX as u64), SignedDuration::MAX);
        assert_eq!(hours(u64::MAX), SignedDuration::MAX);
        assert_eq!(sub(now, minutes(u64::MAX)), UtcTimestamp::MIN);
        assert_eq!(add(now, minutes(u64::MAX)), UtcTimestamp::MAX);
    }

    #[test]
    fn ordinary_values_preserve_duration() {
        let now = UtcTimestamp::now();
        assert_eq!(minutes(30), SignedDuration::from_mins(30));
        assert_eq!(hours(24), SignedDuration::from_hours(24));
        assert_eq!(sub(now, minutes(5)), now - SignedDuration::from_mins(5));
    }
}
