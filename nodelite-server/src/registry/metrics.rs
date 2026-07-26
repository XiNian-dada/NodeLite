//! Argon2 token 验证并发与排队指标。

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct TokenVerifyMetrics {
    pub(crate) limit: u64,
    pub(crate) active: u64,
    pub(crate) waiting: u64,
    pub(crate) wait_seconds_total: f64,
    pub(crate) token_cache_hits_total: u64,
    pub(crate) token_cache_misses_total: u64,
    pub(crate) token_cache_evictions_total: u64,
}

#[derive(Debug, Default)]
pub(super) struct TokenVerifyMetricsState {
    active: AtomicU64,
    waiting: AtomicU64,
    wait_nanos_total: AtomicU64,
    token_cache_hits_total: AtomicU64,
    token_cache_misses_total: AtomicU64,
    token_cache_evictions_total: AtomicU64,
}

impl TokenVerifyMetricsState {
    pub(super) fn snapshot(&self, limit: usize) -> TokenVerifyMetrics {
        TokenVerifyMetrics {
            limit: limit as u64,
            active: self.active.load(Ordering::Relaxed),
            waiting: self.waiting.load(Ordering::Relaxed),
            wait_seconds_total: self.wait_nanos_total.load(Ordering::Relaxed) as f64
                / 1_000_000_000.0,
            token_cache_hits_total: self.token_cache_hits_total.load(Ordering::Relaxed),
            token_cache_misses_total: self.token_cache_misses_total.load(Ordering::Relaxed),
            token_cache_evictions_total: self.token_cache_evictions_total.load(Ordering::Relaxed),
        }
    }

    pub(super) fn record_cache_hit(&self) {
        self.token_cache_hits_total.fetch_add(1, Ordering::Relaxed);
    }

    pub(super) fn record_cache_miss(&self) {
        self.token_cache_misses_total
            .fetch_add(1, Ordering::Relaxed);
    }

    pub(super) fn record_cache_eviction(&self) {
        self.token_cache_evictions_total
            .fetch_add(1, Ordering::Relaxed);
    }

    fn record_wait(&self, waited: Duration) {
        let nanos = u64::try_from(waited.as_nanos()).unwrap_or(u64::MAX);
        let _ =
            self.wait_nanos_total
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
                    Some(current.saturating_add(nanos))
                });
    }
}

pub(super) struct TokenVerifyWaitingGuard {
    state: Arc<TokenVerifyMetricsState>,
    started: Instant,
    waiting: bool,
}

impl TokenVerifyWaitingGuard {
    pub(super) fn start(state: Arc<TokenVerifyMetricsState>) -> Self {
        state.waiting.fetch_add(1, Ordering::Relaxed);
        Self {
            state,
            started: Instant::now(),
            waiting: true,
        }
    }

    pub(super) fn acquired(mut self) -> Duration {
        let waited = self.started.elapsed();
        self.state.record_wait(waited);
        self.state.waiting.fetch_sub(1, Ordering::Relaxed);
        self.waiting = false;
        waited
    }
}

impl Drop for TokenVerifyWaitingGuard {
    fn drop(&mut self) {
        if self.waiting {
            self.state.record_wait(self.started.elapsed());
            self.state.waiting.fetch_sub(1, Ordering::Relaxed);
        }
    }
}

pub(super) struct TokenVerifyActiveGuard {
    state: Arc<TokenVerifyMetricsState>,
}

impl TokenVerifyActiveGuard {
    pub(super) fn start(state: Arc<TokenVerifyMetricsState>) -> Self {
        state.active.fetch_add(1, Ordering::Relaxed);
        Self { state }
    }
}

impl Drop for TokenVerifyActiveGuard {
    fn drop(&mut self) {
        self.state.active.fetch_sub(1, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guards_restore_gauges_when_work_is_cancelled_or_completed() {
        let state = Arc::new(TokenVerifyMetricsState::default());
        {
            let _waiting = TokenVerifyWaitingGuard::start(Arc::clone(&state));
            let _active = TokenVerifyActiveGuard::start(Arc::clone(&state));
            let snapshot = state.snapshot(4);
            assert_eq!(snapshot.waiting, 1);
            assert_eq!(snapshot.active, 1);
        }

        let snapshot = state.snapshot(4);
        assert_eq!(snapshot.waiting, 0);
        assert_eq!(snapshot.active, 0);
        assert!(snapshot.wait_seconds_total > 0.0);
    }
}
