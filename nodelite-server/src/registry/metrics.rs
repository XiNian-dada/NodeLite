//! Argon2 token 验证并发与排队指标。

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TokenVerifyMetrics {
    pub limit: u64,
    pub active: u64,
    pub waiting: u64,
    pub wait_seconds_total: f64,
    pub token_cache_hits_total: u64,
    pub token_cache_misses_total: u64,
    pub token_cache_evictions_total: u64,
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
        let mut current = self.wait_nanos_total.load(Ordering::Relaxed);
        loop {
            match self.wait_nanos_total.compare_exchange_weak(
                current,
                current.saturating_add(nanos),
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(actual) => current = actual,
            }
        }
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
    fn wait_time_accumulates_and_saturates_without_overflow() {
        let state = TokenVerifyMetricsState::default();
        state.record_wait(Duration::ZERO);
        state.record_wait(Duration::from_nanos(2));
        state.record_wait(Duration::from_nanos(3));
        assert_eq!(state.wait_nanos_total.load(Ordering::Relaxed), 5);

        state
            .wait_nanos_total
            .store(u64::MAX - 1, Ordering::Relaxed);
        state.record_wait(Duration::from_nanos(2));
        assert_eq!(state.wait_nanos_total.load(Ordering::Relaxed), u64::MAX);

        state.wait_nanos_total.store(0, Ordering::Relaxed);
        state.record_wait(Duration::MAX);
        state.record_wait(Duration::from_nanos(1));
        assert_eq!(state.wait_nanos_total.load(Ordering::Relaxed), u64::MAX);
    }

    #[test]
    fn concurrent_wait_times_do_not_lose_updates() {
        let state = Arc::new(TokenVerifyMetricsState::default());
        let barrier = Arc::new(std::sync::Barrier::new(8));
        let workers: Vec<_> = (0..8)
            .map(|_| {
                let state = Arc::clone(&state);
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    barrier.wait();
                    for _ in 0..1_000 {
                        state.record_wait(Duration::from_nanos(1));
                    }
                })
            })
            .collect();
        for worker in workers {
            worker.join().expect("metric worker should finish");
        }
        assert_eq!(state.wait_nanos_total.load(Ordering::Relaxed), 8_000);
    }

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
