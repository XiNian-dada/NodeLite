//! 历史只读查询的并发限制与等待指标。

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use tokio::sync::{AcquireError, OwnedSemaphorePermit, Semaphore, TryAcquireError};

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct HistoryQueryRuntimeMetrics {
    pub(crate) permits_in_use: u64,
    pub(crate) waiting: u64,
    pub(crate) limit: u64,
    pub(crate) acquisitions_total: u64,
    pub(crate) waits_total: u64,
    pub(crate) wait_seconds_total: f64,
}

#[derive(Clone)]
pub(super) struct HistoryQueryLimiter {
    limit: usize,
    semaphore: Arc<Semaphore>,
    permits_in_use: Arc<AtomicU64>,
    waiting: Arc<AtomicU64>,
    acquisitions_total: Arc<AtomicU64>,
    waits_total: Arc<AtomicU64>,
    wait_nanos_total: Arc<AtomicU64>,
}

impl HistoryQueryLimiter {
    pub(super) fn new(limit: usize) -> Self {
        let limit = limit.max(1);
        Self {
            limit,
            semaphore: Arc::new(Semaphore::new(limit)),
            permits_in_use: Arc::new(AtomicU64::new(0)),
            waiting: Arc::new(AtomicU64::new(0)),
            acquisitions_total: Arc::new(AtomicU64::new(0)),
            waits_total: Arc::new(AtomicU64::new(0)),
            wait_nanos_total: Arc::new(AtomicU64::new(0)),
        }
    }

    pub(super) async fn acquire(&self) -> Result<HistoryQueryPermit, AcquireError> {
        let semaphore = Arc::clone(&self.semaphore);
        let permit = match Arc::clone(&semaphore).try_acquire_owned() {
            Ok(permit) => permit,
            Err(TryAcquireError::NoPermits) => {
                self.waits_total.fetch_add(1, Ordering::Relaxed);
                let waiting_guard = WaitingQueryGuard::new(
                    Arc::clone(&self.waiting),
                    Arc::clone(&self.wait_nanos_total),
                );
                let permit = semaphore.acquire_owned().await?;
                drop(waiting_guard);
                permit
            }
            Err(TryAcquireError::Closed) => semaphore.acquire_owned().await?,
        };
        self.acquisitions_total.fetch_add(1, Ordering::Relaxed);
        Ok(HistoryQueryPermit::new(
            permit,
            Arc::clone(&self.permits_in_use),
        ))
    }

    pub(super) fn metrics(&self) -> HistoryQueryRuntimeMetrics {
        HistoryQueryRuntimeMetrics {
            permits_in_use: self.permits_in_use.load(Ordering::Relaxed),
            waiting: self.waiting.load(Ordering::Relaxed),
            limit: self.limit as u64,
            acquisitions_total: self.acquisitions_total.load(Ordering::Relaxed),
            waits_total: self.waits_total.load(Ordering::Relaxed),
            wait_seconds_total: self.wait_nanos_total.load(Ordering::Relaxed) as f64
                / 1_000_000_000.0,
        }
    }
}

struct WaitingQueryGuard {
    waiting: Arc<AtomicU64>,
    wait_nanos_total: Arc<AtomicU64>,
    started_at: Instant,
}

impl WaitingQueryGuard {
    fn new(waiting: Arc<AtomicU64>, wait_nanos_total: Arc<AtomicU64>) -> Self {
        waiting.fetch_add(1, Ordering::Relaxed);
        Self {
            waiting,
            wait_nanos_total,
            started_at: Instant::now(),
        }
    }
}

impl Drop for WaitingQueryGuard {
    fn drop(&mut self) {
        self.waiting.fetch_sub(1, Ordering::Relaxed);
        let waited_nanos = u64::try_from(self.started_at.elapsed().as_nanos()).unwrap_or(u64::MAX);
        self.wait_nanos_total
            .fetch_add(waited_nanos, Ordering::Relaxed);
    }
}

pub(super) struct HistoryQueryPermit {
    _permit: OwnedSemaphorePermit,
    permits_in_use: Arc<AtomicU64>,
}

impl HistoryQueryPermit {
    fn new(permit: OwnedSemaphorePermit, permits_in_use: Arc<AtomicU64>) -> Self {
        permits_in_use.fetch_add(1, Ordering::Relaxed);
        Self {
            _permit: permit,
            permits_in_use,
        }
    }
}

impl Drop for HistoryQueryPermit {
    fn drop(&mut self) {
        self.permits_in_use.fetch_sub(1, Ordering::Relaxed);
    }
}
