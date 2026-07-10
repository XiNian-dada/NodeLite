//! 历史查询结果缓存及其内存预算。

use std::mem::size_of;
use std::num::NonZeroUsize;
use std::time::{Duration, Instant};

use lru::LruCache;
use nodelite_proto::HistoryPoint;

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(super) struct CacheKey {
    node_id: String,
    since_ts: i64,
    until_ts: i64,
    max_points: usize,
}

impl CacheKey {
    pub(super) fn new(node_id: &str, since_ts: i64, until_ts: i64, max_points: usize) -> Self {
        Self {
            node_id: node_id.to_string(),
            since_ts,
            until_ts,
            max_points,
        }
    }

    fn estimated_bytes(&self) -> usize {
        size_of::<Self>().saturating_add(self.node_id.capacity())
    }
}

struct CacheEntry {
    points: Vec<HistoryPoint>,
    cached_at: Instant,
    estimated_bytes: usize,
}

impl CacheEntry {
    fn new(key: &CacheKey, points: Vec<HistoryPoint>, cached_at: Instant) -> Self {
        let point_allocations = points
            .iter()
            .map(|point| point.node_id.capacity())
            .sum::<usize>();
        let estimated_bytes = key
            .estimated_bytes()
            .saturating_add(size_of::<Self>())
            .saturating_add(points.capacity().saturating_mul(size_of::<HistoryPoint>()))
            .saturating_add(point_allocations);
        Self {
            points,
            cached_at,
            estimated_bytes,
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct HistoryCacheMetrics {
    pub(crate) entries: u64,
    pub(crate) estimated_bytes: u64,
    pub(crate) evictions: u64,
    pub(crate) expired_removals: u64,
}

pub(super) struct HistoryQueryCache {
    entries: LruCache<CacheKey, CacheEntry>,
    max_bytes: usize,
    ttl: Duration,
    estimated_bytes: usize,
    evictions: u64,
    expired_removals: u64,
}

impl HistoryQueryCache {
    pub(super) fn new(capacity: NonZeroUsize, max_bytes: usize, ttl: Duration) -> Self {
        Self {
            entries: LruCache::new(capacity),
            max_bytes,
            ttl,
            estimated_bytes: 0,
            evictions: 0,
            expired_removals: 0,
        }
    }

    pub(super) fn get(&mut self, key: &CacheKey, now: Instant) -> Option<Vec<HistoryPoint>> {
        self.prune_expired(now);
        self.entries.get(key).map(|entry| entry.points.clone())
    }

    pub(super) fn insert(&mut self, key: CacheKey, points: Vec<HistoryPoint>, now: Instant) {
        self.prune_expired(now);
        let entry = CacheEntry::new(&key, points, now);
        if entry.estimated_bytes > self.max_bytes {
            return;
        }

        self.estimated_bytes = self.estimated_bytes.saturating_add(entry.estimated_bytes);
        if let Some((removed_key, removed_entry)) = self.entries.push(key.clone(), entry) {
            self.estimated_bytes = self
                .estimated_bytes
                .saturating_sub(removed_entry.estimated_bytes);
            if removed_key != key {
                self.evictions = self.evictions.saturating_add(1);
            }
        }

        while self.estimated_bytes > self.max_bytes {
            let Some((_, removed_entry)) = self.entries.pop_lru() else {
                self.estimated_bytes = 0;
                break;
            };
            self.estimated_bytes = self
                .estimated_bytes
                .saturating_sub(removed_entry.estimated_bytes);
            self.evictions = self.evictions.saturating_add(1);
        }
    }

    pub(super) fn metrics(&self) -> HistoryCacheMetrics {
        HistoryCacheMetrics {
            entries: self.entries.len() as u64,
            estimated_bytes: self.estimated_bytes as u64,
            evictions: self.evictions,
            expired_removals: self.expired_removals,
        }
    }

    pub(super) fn prune_expired(&mut self, now: Instant) -> usize {
        let expired_keys = self
            .entries
            .iter()
            .filter(|(_, entry)| now.saturating_duration_since(entry.cached_at) >= self.ttl)
            .map(|(key, _)| key.clone())
            .collect::<Vec<_>>();
        let expired_count = expired_keys.len();
        for key in expired_keys {
            if let Some((_, removed_entry)) = self.entries.pop_entry(&key) {
                self.estimated_bytes = self
                    .estimated_bytes
                    .saturating_sub(removed_entry.estimated_bytes);
                self.expired_removals = self.expired_removals.saturating_add(1);
            }
        }
        expired_count
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use super::*;

    fn point(node_id: &str) -> HistoryPoint {
        HistoryPoint {
            node_id: node_id.to_string(),
            recorded_at: Utc::now(),
            cpu_usage_percent: Some(1.0),
            load_one: Some(0.1),
            load_five: Some(0.2),
            load_fifteen: Some(0.3),
            memory_used_percent: 2.0,
            rx_bytes_per_sec: Some(3.0),
            tx_bytes_per_sec: Some(4.0),
            latency_ms: Some(5),
            packet_loss_percent: Some(0.5),
            disk_used_percent: Some(6.0),
        }
    }

    #[test]
    fn access_prunes_all_expired_entries() {
        let ttl = Duration::from_secs(1);
        let start = Instant::now();
        let mut cache = HistoryQueryCache::new(
            NonZeroUsize::new(10).expect("test capacity should be non-zero"),
            usize::MAX,
            ttl,
        );
        let first = CacheKey::new("first", 1, 2, 60);
        let second = CacheKey::new("second", 1, 2, 60);
        cache.insert(first, vec![point("first")], start);
        cache.insert(second, vec![point("second")], start);

        let missing = CacheKey::new("missing", 1, 2, 60);
        assert!(cache.get(&missing, start + ttl).is_none());
        let metrics = cache.metrics();
        assert_eq!(metrics.entries, 0);
        assert_eq!(metrics.estimated_bytes, 0);
        assert_eq!(metrics.expired_removals, 2);
    }

    #[test]
    fn maintenance_prunes_expired_entries_without_a_followup_query() {
        let ttl = Duration::from_secs(1);
        let start = Instant::now();
        let mut cache = HistoryQueryCache::new(
            NonZeroUsize::new(10).expect("test capacity should be non-zero"),
            usize::MAX,
            ttl,
        );
        cache.insert(CacheKey::new("idle", 1, 2, 60), vec![point("idle")], start);

        assert_eq!(cache.prune_expired(start + ttl), 1);
        assert_eq!(cache.metrics().entries, 0);
        assert_eq!(cache.metrics().expired_removals, 1);
    }

    #[test]
    fn insertion_evicts_lru_entries_to_stay_within_byte_budget() {
        let start = Instant::now();
        let sample_key = CacheKey::new("sample", 1, 2, 60);
        let sample_entry = CacheEntry::new(&sample_key, vec![point("sample")], start);
        let budget = sample_entry.estimated_bytes.saturating_mul(2);
        let mut cache = HistoryQueryCache::new(
            NonZeroUsize::new(10).expect("test capacity should be non-zero"),
            budget,
            Duration::from_secs(60),
        );

        for node_id in ["first", "second", "third"] {
            cache.insert(
                CacheKey::new(node_id, 1, 2, 60),
                vec![point(node_id)],
                start,
            );
        }

        let metrics = cache.metrics();
        assert!(metrics.entries <= 2);
        assert!(metrics.estimated_bytes <= budget as u64);
        assert!(metrics.evictions >= 1);
        assert!(
            cache
                .get(&CacheKey::new("first", 1, 2, 60), start)
                .is_none()
        );
    }

    #[test]
    fn entry_larger_than_budget_is_not_cached() {
        let start = Instant::now();
        let mut cache = HistoryQueryCache::new(
            NonZeroUsize::new(10).expect("test capacity should be non-zero"),
            1,
            Duration::from_secs(60),
        );
        let key = CacheKey::new("oversized", 1, 2, 60);
        cache.insert(key.clone(), vec![point("oversized")], start);

        assert!(cache.get(&key, start).is_none());
        assert_eq!(cache.metrics().entries, 0);
    }
}
