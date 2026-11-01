//! 限流事件的审计写入节流。
//!
//! 封禁期内每个被拒请求都会产生一条 `rate_limit_exceeded`，审计库增长会与请求数成正比。
//! 同一来源与原因在窗口内只写一条，下一条记录带上期间省略的次数。

use std::collections::HashMap;
use std::net::{IpAddr, Ipv6Addr};
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub(super) const RATE_LIMIT_AUDIT_WINDOW: Duration = Duration::from_secs(60);
/// Tracking stays bounded even when many sources are blocked at once.
const MAX_TRACKED_SOURCES: usize = 4096;

/// One IPv6 client usually controls a whole /64, so per-address keys would not throttle it.
pub(super) fn throttle_source(ip_address: &str) -> String {
    match ip_address.parse::<IpAddr>() {
        Ok(IpAddr::V6(address)) if address.to_ipv4_mapped().is_none() => {
            let prefix = u128::from(address) & !((1u128 << 64) - 1);
            format!("{}/64", Ipv6Addr::from(prefix))
        }
        _ => ip_address.to_string(),
    }
}

struct Entry {
    last_recorded: Instant,
    suppressed: u64,
}

#[derive(Default)]
pub(super) struct RateLimitAuditThrottle {
    entries: Mutex<HashMap<(String, String), Entry>>,
}

impl RateLimitAuditThrottle {
    /// Returns `Some(suppressed)` when the event should be stored, `None` to skip it.
    pub(super) fn admit(&self, source: &str, reason: &str, now: Instant) -> Option<u64> {
        let mut entries = self
            .entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let key = (source.to_string(), reason.to_string());
        if let Some(entry) = entries.get_mut(&key) {
            if now.duration_since(entry.last_recorded) < RATE_LIMIT_AUDIT_WINDOW {
                entry.suppressed = entry.suppressed.saturating_add(1);
                return None;
            }
            let suppressed = entry.suppressed;
            entry.last_recorded = now;
            entry.suppressed = 0;
            return Some(suppressed);
        }
        if entries.len() >= MAX_TRACKED_SOURCES {
            entries.retain(|_, entry| {
                now.duration_since(entry.last_recorded) < RATE_LIMIT_AUDIT_WINDOW
            });
        }
        // Still full: record without tracking rather than lose the event.
        if entries.len() < MAX_TRACKED_SOURCES {
            entries.insert(
                key,
                Entry {
                    last_recorded: now,
                    suppressed: 0,
                },
            );
        }
        Some(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeats_inside_the_window_are_counted_and_reported_once() {
        let throttle = RateLimitAuditThrottle::default();
        let start = Instant::now();
        assert_eq!(throttle.admit("198.51.100.1", "ws", start), Some(0));
        for second in 1..=5 {
            let at = start + Duration::from_secs(second);
            assert_eq!(throttle.admit("198.51.100.1", "ws", at), None);
        }
        assert_eq!(
            throttle.admit("198.51.100.2", "ws", start),
            Some(0),
            "other sources are independent"
        );
        assert_eq!(
            throttle.admit("198.51.100.1", "install", start),
            Some(0),
            "other reasons are independent"
        );
        let later = start + RATE_LIMIT_AUDIT_WINDOW;
        assert_eq!(throttle.admit("198.51.100.1", "ws", later), Some(5));
        assert_eq!(throttle.admit("198.51.100.1", "ws", later), None);
    }

    #[test]
    fn ipv6_sources_are_grouped_by_their_64_prefix() {
        assert_eq!(throttle_source("2001:db8:1:2:aaaa::1"), "2001:db8:1:2::/64");
        assert_eq!(
            throttle_source("2001:db8:1:2:bbbb::9"),
            throttle_source("2001:db8:1:2:aaaa::1")
        );
        assert_ne!(
            throttle_source("2001:db8:1:3::1"),
            throttle_source("2001:db8:1:2::1")
        );
        assert_eq!(throttle_source("198.51.100.7"), "198.51.100.7");
        assert_eq!(
            throttle_source("::ffff:198.51.100.7"),
            "::ffff:198.51.100.7"
        );
        assert_eq!(throttle_source("not-an-ip"), "not-an-ip");
    }

    #[test]
    fn tracking_is_bounded_and_expired_entries_are_reclaimed() {
        let throttle = RateLimitAuditThrottle::default();
        let start = Instant::now();
        for index in 0..MAX_TRACKED_SOURCES {
            assert_eq!(throttle.admit(&index.to_string(), "ws", start), Some(0));
        }
        assert_eq!(throttle.admit("overflow", "ws", start), Some(0));
        assert_eq!(
            throttle.admit("overflow", "ws", start),
            Some(0),
            "untracked sources are still recorded"
        );
        let later = start + RATE_LIMIT_AUDIT_WINDOW;
        assert_eq!(throttle.admit("fresh", "ws", later), Some(0));
        assert_eq!(throttle.admit("fresh", "ws", later), None);
        let entries = throttle.entries.lock().expect("lock");
        assert_eq!(entries.len(), 1);
    }
}
