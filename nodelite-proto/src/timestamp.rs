//! UTC instants with the RFC 3339 representation used by the wire protocol and registry.

use std::fmt;
use std::ops::{Add, Sub};
use std::str::FromStr;

use jiff::civil::{Date, DateTime};
use jiff::fmt::temporal::Pieces;
use jiff::tz::Offset;
use jiff::{SignedDuration, Timestamp};
use serde::{Deserialize, Deserializer, Serialize, Serializer, de};

/// An absolute timestamp with UTC serialization and nanosecond precision.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct UtcTimestamp(Timestamp);

impl UtcTimestamp {
    pub const MIN: Self = Self(Timestamp::MIN);
    pub const MAX: Self = Self(Timestamp::MAX);

    pub fn now() -> Self {
        Self(Timestamp::now())
    }

    pub fn from_second(seconds: i64) -> Option<Self> {
        Timestamp::from_second(seconds).map(Self).ok()
    }

    pub fn from_utc(
        year: i16,
        month: i8,
        day: i8,
        hour: i8,
        minute: i8,
        second: i8,
    ) -> Option<Self> {
        let datetime = DateTime::new(year, month, day, hour, minute, second, 0).ok()?;
        Offset::UTC.to_timestamp(datetime).map(Self).ok()
    }

    pub fn timestamp(self) -> i64 {
        // Unix seconds and milliseconds round towards the past, including before 1970.
        self.0.as_second() - i64::from(self.0.subsec_nanosecond() < 0)
    }

    pub fn timestamp_millis(self) -> i64 {
        self.0.as_millisecond() - i64::from(self.0.subsec_nanosecond() % 1_000_000 < 0)
    }

    pub fn as_nanosecond(self) -> i128 {
        self.0.as_nanosecond()
    }

    pub fn utc_date(self) -> Date {
        Offset::UTC.to_datetime(self.0).date()
    }

    pub fn utc_month_start(self) -> Self {
        let date = self.utc_date();
        Self::from_utc(date.year(), date.month(), 1, 0, 0, 0).unwrap_or(self)
    }

    pub fn saturating_add(self, duration: SignedDuration) -> Self {
        self.0.checked_add(duration).map(Self).unwrap_or_else(|_| {
            if duration.is_negative() {
                Self::MIN
            } else {
                Self::MAX
            }
        })
    }

    pub fn saturating_sub(self, duration: SignedDuration) -> Self {
        self.0.checked_sub(duration).map(Self).unwrap_or_else(|_| {
            if duration.is_negative() {
                Self::MAX
            } else {
                Self::MIN
            }
        })
    }

    pub fn strftime<'a>(&self, format: &'a str) -> jiff::fmt::strtime::Display<'a> {
        self.0.strftime(format)
    }

    /// Format a UTC timestamp for API fields that already use a numeric offset.
    pub fn to_rfc3339(self) -> String {
        self.format_with_offset(Offset::UTC)
    }

    fn format_with_offset(self, offset: Offset) -> String {
        let precision = fraction_digits(self.0);
        format!("{:.*}", precision, self.0.display_with_offset(offset))
    }
}

/// Preserve the offset provided by an agent when normalizing a log timestamp.
pub fn normalize_log_timestamp(value: &str) -> Option<String> {
    let instant = value.parse::<UtcTimestamp>().ok()?;
    let offset = Pieces::parse(value).ok()?.offset()?.to_numeric_offset();
    Some(instant.format_with_offset(offset))
}

fn fraction_digits(timestamp: Timestamp) -> usize {
    let nanos = timestamp.subsec_nanosecond();
    if nanos == 0 {
        0
    } else if nanos % 1_000_000 == 0 {
        3
    } else if nanos % 1_000 == 0 {
        6
    } else {
        9
    }
}

impl FromStr for UtcTimestamp {
    type Err = jiff::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        value.parse().map(Self)
    }
}

impl fmt::Display for UtcTimestamp {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{:.*}", fraction_digits(self.0), self.0)
    }
}

impl Serialize for UtcTimestamp {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for UtcTimestamp {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct TimestampVisitor;

        impl de::Visitor<'_> for TimestampVisitor {
            type Value = UtcTimestamp;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("an RFC 3339 timestamp with a UTC offset")
            }

            fn visit_str<E: de::Error>(self, value: &str) -> Result<UtcTimestamp, E> {
                value.parse().map_err(E::custom)
            }
        }

        deserializer.deserialize_str(TimestampVisitor)
    }
}

impl Add<SignedDuration> for UtcTimestamp {
    type Output = Self;

    fn add(self, duration: SignedDuration) -> Self {
        self.saturating_add(duration)
    }
}

impl Sub<SignedDuration> for UtcTimestamp {
    type Output = Self;

    fn sub(self, duration: SignedDuration) -> Self {
        self.saturating_sub(duration)
    }
}

impl Sub for UtcTimestamp {
    type Output = SignedDuration;

    fn sub(self, earlier: Self) -> SignedDuration {
        self.0.duration_since(earlier.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_wire_timestamp_precision() {
        let cases = [
            ("2026-11-20T00:00:00Z", "2026-11-20T00:00:00Z"),
            ("2026-11-20T08:00:00.1+08:00", "2026-11-20T00:00:00.100Z"),
            ("2026-11-20T00:00:00.0001Z", "2026-11-20T00:00:00.000100Z"),
            (
                "2026-11-20T00:00:00.0000001Z",
                "2026-11-20T00:00:00.000000100Z",
            ),
        ];
        for (input, expected) in cases {
            let time: UtcTimestamp = serde_json::from_value(serde_json::json!(input)).unwrap();
            assert_eq!(serde_json::to_value(time).unwrap(), expected);
            assert_eq!(
                time.to_rfc3339(),
                format!("{}+00:00", expected.trim_end_matches('Z'))
            );
        }
        assert!(serde_json::from_str::<UtcTimestamp>("\"2026-11-20T00:00:00\"").is_err());
    }

    #[test]
    fn preserves_log_offsets() {
        assert_eq!(
            normalize_log_timestamp("2026-11-20T08:00:00.1+08:00").as_deref(),
            Some("2026-11-20T08:00:00.100+08:00")
        );
        assert!(normalize_log_timestamp("invalid").is_none());
    }

    #[test]
    fn floors_unix_seconds_and_milliseconds_before_epoch() {
        let time = "1969-12-31T23:59:59.999999999Z"
            .parse::<UtcTimestamp>()
            .unwrap();
        assert_eq!(time.timestamp(), -1);
        assert_eq!(time.timestamp_millis(), -1);
        assert_eq!(
            UtcTimestamp::from_second(-1).unwrap().timestamp_millis(),
            -1000
        );
    }

    #[test]
    fn month_starts_at_midnight_in_utc() {
        let time = "2026-01-01T01:00:00+08:00".parse::<UtcTimestamp>().unwrap();
        assert_eq!(time.utc_month_start().to_string(), "2025-12-01T00:00:00Z");
    }
}
