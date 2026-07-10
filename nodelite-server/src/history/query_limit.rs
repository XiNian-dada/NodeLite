//! 历史只读查询的并发限制与等待指标。

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use tokio::sync::{AcquireError, OwnedSemaphorePermit, Semaphore};

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct HistoryQueryRuntimeMetrics {
    pub(crate) active: u64,
    pub(crate) waiting: u64,
    pub(crate) limit: u64,
    pub(crate) wait_total: u64,
    pub(crate) wait_seconds_total: f64,
}

#[derive(Clone)]
pub(super) struct HistoryQueryLimiter {
    limit: usize,
    semaphore: Arc<Semaphore>,
    active: Arc<AtomicU64>,
    waiting: Arc<AtomicU64>,
    wait_total: Arc<AtomicU64>,
    wait_nanos_total: Arc<AtomicU64>,
}

impl HistoryQueryLimiter {
    pub(super) fn new(limit: usize) -> Self {
        let limit = limit.max(1);
        Self {
            limit,
            semaphore: Arc::new(Semaphore::new(limit)),
            active: Arc::new(AtomicU64::new(0)),
            waiting: Arc::new(AtomicU64::new(0)),
            wait_total: Arc::new(AtomicU64::new(0)),
            wait_nanos_total: Arc::new(AtomicU64::new(0)),
        }
    }

    pub(super) async fn acquire(&self) -> Result<HistoryQueryPermit, AcquireError> {
        let waiting_guard = WaitingQueryGuard::new(Arc::clone(&self.waiting));
        let wait_started = Instant::now();
        let permit = Arc::clone(&self.semaphore).acquire_owned().await?;
        let waited = wait_started.elapsed();
        drop(waiting_guard);
        self.wait_total.fetch_add(1, Ordering::Relaxed);
        let waited_nanos = u64::try_from(waited.as_nanos()).unwrap_or(u64::MAX);
        self.wait_nanos_total
            .fetch_add(waited_nanos, Ordering::Relaxed);
        Ok(HistoryQueryPermit::new(permit, Arc::clone(&self.active)))
    }

    pub(super) fn metrics(&self) -> HistoryQueryRuntimeMetrics {
        HistoryQueryRuntimeMetrics {
            active: self.active.load(Ordering::Relaxed),
            waiting: self.waiting.load(Ordering::Relaxed),
            limit: self.limit as u64,
            wait_total: self.wait_total.load(Ordering::Relaxed),
            wait_seconds_total: self.wait_nanos_total.load(Ordering::Relaxed) as f64
                / 1_000_000_000.0,
        }
    }
}

struct WaitingQueryGuard {
    waiting: Arc<AtomicU64>,
}

impl WaitingQueryGuard {
    fn new(waiting: Arc<AtomicU64>) -> Self {
        waiting.fetch_add(1, Ordering::Relaxed);
        Self { waiting }
    }
}

impl Drop for WaitingQueryGuard {
    fn drop(&mut self) {
        self.waiting.fetch_sub(1, Ordering::Relaxed);
    }
}

pub(super) struct HistoryQueryPermit {
    _permit: OwnedSemaphorePermit,
    active: Arc<AtomicU64>,
}

impl HistoryQueryPermit {
    fn new(permit: OwnedSemaphorePermit, active: Arc<AtomicU64>) -> Self {
        active.fetch_add(1, Ordering::Relaxed);
        Self {
            _permit: permit,
            active,
        }
    }
}

impl Drop for HistoryQueryPermit {
    fn drop(&mut self) {
        self.active.fetch_sub(1, Ordering::Relaxed);
    }
}
