//! 概览 API 与 Prometheus 输出的瞬时缓存。

use std::time::{Duration, Instant};

use axum::body::Bytes;

use crate::ServerReadiness;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ReadinessSnapshot {
    ready: bool,
    history_available: bool,
    registry_reload_healthy: bool,
}

impl ReadinessSnapshot {
    pub(super) fn new(ready: bool, history_available: bool, registry_reload_healthy: bool) -> Self {
        Self {
            ready,
            history_available,
            registry_reload_healthy,
        }
    }

    pub(super) fn capture(readiness: &ServerReadiness) -> Self {
        Self::new(
            readiness.is_ready(),
            readiness.history_available(),
            readiness.registry_reload_healthy(),
        )
    }
}

/// 简单 JSON 视图(overview / nodes)的缓存槽:可选 TTL 校验。
///
/// `max_age = None` 时退化为纯 revision 校验(nodes 视图的当前行为);
/// `max_age = Some(d)` 时除 revision 外还要求 cached_at + d > now,
/// 用于 overview 在 revision 长期不变时仍能定期重建,避免聚合数据无限陈旧。
#[derive(Debug, Default)]
pub(super) struct JsonViewSlot {
    revision: u64,
    cached_at: Option<Instant>,
    body: Option<Bytes>,
}

impl JsonViewSlot {
    pub(super) fn get(&self, revision: u64, max_age: Option<Duration>) -> Option<Bytes> {
        // TTL 优先:在 TTL 内直接返回,忽略 revision 变化(高频上报时容忍短暂陈旧以提升命中率)
        if let Some(max_age) = max_age {
            let cached_at = self.cached_at?;
            if cached_at.elapsed() < max_age {
                return self.body.clone();
            }
        }
        // TTL 过期或未设置时才严格比较 revision
        if self.revision != revision {
            return None;
        }
        self.body.clone()
    }

    pub(super) fn store(&mut self, revision: u64, body: Bytes) {
        self.revision = revision;
        self.cached_at = Some(Instant::now());
        self.body = Some(body);
    }
}

/// Prometheus `/metrics` 文本的缓存槽:revision、readiness 与 TTL 三重校验。
#[derive(Debug, Default)]
pub(super) struct MetricsViewSlot {
    revision: u64,
    readiness: Option<ReadinessSnapshot>,
    cached_at: Option<Instant>,
    body: Option<Bytes>,
}

impl MetricsViewSlot {
    pub(super) fn get(
        &self,
        revision: u64,
        readiness: ReadinessSnapshot,
        max_age: Duration,
    ) -> Option<Bytes> {
        if self.revision != revision {
            return None;
        }
        if self.readiness != Some(readiness) {
            return None;
        }
        if self
            .cached_at
            .map(|cached_at| cached_at.elapsed() > max_age)
            .unwrap_or(true)
        {
            return None;
        }
        self.body.clone()
    }

    pub(super) fn store(&mut self, revision: u64, readiness: ReadinessSnapshot, body: Bytes) {
        self.revision = revision;
        self.readiness = Some(readiness);
        self.cached_at = Some(Instant::now());
        self.body = Some(body);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_view_slot_misses_when_revision_changes_and_no_ttl() {
        let mut slot = JsonViewSlot::default();
        slot.store(7, Bytes::from_static(b"body"));
        assert_eq!(slot.get(7, None).as_deref(), Some(b"body".as_ref()));
        // Without TTL, revision mismatch causes miss.
        assert!(slot.get(8, None).is_none());
    }

    #[test]
    fn json_view_slot_tolerates_revision_change_within_ttl() {
        let mut slot = JsonViewSlot::default();
        slot.store(1, Bytes::from_static(b"body"));
        // Within TTL, revision change is tolerated.
        assert!(slot.get(2, Some(Duration::from_secs(60))).is_some());
        assert_eq!(
            slot.get(99, Some(Duration::from_secs(60))).as_deref(),
            Some(b"body".as_ref())
        );
    }

    #[test]
    fn json_view_slot_expires_after_max_age() {
        let mut slot = JsonViewSlot::default();
        slot.store(1, Bytes::from_static(b"body"));
        // Fresh body within TTL hits.
        assert!(slot.get(1, Some(Duration::from_secs(60))).is_some());
        // After waiting past the TTL, the cache should miss even with same revision.
        std::thread::sleep(Duration::from_millis(20));
        assert!(slot.get(1, Some(Duration::from_millis(5))).is_none());
        // Pure revision check still hits.
        assert!(slot.get(1, None).is_some());
    }
}
