//! Saturating time arithmetic for config-driven alert durations.
//!
//! Config parsing already bounds these values; saturating here keeps a value that bypassed
//! parsing (e.g. a future settings path) from panicking the alert task inside chrono.

use chrono::{DateTime, TimeDelta, Utc};

pub(crate) fn minutes(value: u64) -> TimeDelta {
    i64::try_from(value)
        .ok()
        .and_then(TimeDelta::try_minutes)
        .unwrap_or(TimeDelta::MAX)
}

pub(crate) fn hours(value: u64) -> TimeDelta {
    i64::try_from(value)
        .ok()
        .and_then(TimeDelta::try_hours)
        .unwrap_or(TimeDelta::MAX)
}

pub(crate) fn sub(at: DateTime<Utc>, delta: TimeDelta) -> DateTime<Utc> {
    at.checked_sub_signed(delta)
        .unwrap_or(DateTime::<Utc>::MIN_UTC)
}

pub(crate) fn add(at: DateTime<Utc>, delta: TimeDelta) -> DateTime<Utc> {
    at.checked_add_signed(delta)
        .unwrap_or(DateTime::<Utc>::MAX_UTC)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oversized_values_saturate_instead_of_panicking() {
        let now = Utc::now();
        assert_eq!(minutes(u64::MAX), TimeDelta::MAX);
        assert_eq!(minutes(i64::MAX as u64), TimeDelta::MAX);
        assert_eq!(hours(u64::MAX), TimeDelta::MAX);
        assert_eq!(sub(now, minutes(u64::MAX)), DateTime::<Utc>::MIN_UTC);
        assert_eq!(add(now, minutes(u64::MAX)), DateTime::<Utc>::MAX_UTC);
    }

    #[test]
    fn ordinary_values_match_chrono() {
        let now = Utc::now();
        assert_eq!(minutes(30), TimeDelta::minutes(30));
        assert_eq!(hours(24), TimeDelta::hours(24));
        assert_eq!(sub(now, minutes(5)), now - TimeDelta::minutes(5));
    }
}
