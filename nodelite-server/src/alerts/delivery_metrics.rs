//! Count a delivery until its result is consumed, including queued and cancelled work.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

pub(crate) const DELIVERY_QUEUE_CAPACITY: usize = 1024;
pub(crate) const MAX_CONCURRENT_DELIVERIES: usize = 8;
pub(crate) const DELIVERY_RESULT_CAPACITY: usize = 8;
pub(crate) const DELIVERY_TOTAL_CAPACITY: usize =
    DELIVERY_QUEUE_CAPACITY + MAX_CONCURRENT_DELIVERIES + DELIVERY_RESULT_CAPACITY;

#[derive(Clone, Debug, Default)]
pub(crate) struct AlertDeliveryMetrics {
    outstanding: Arc<AtomicU64>,
    active: Arc<AtomicU64>,
    queue_full: Arc<AtomicU64>,
    queue_closed: Arc<AtomicU64>,
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct AlertDeliverySnapshot {
    pub(crate) outstanding: u64,
    pub(crate) active: u64,
    pub(crate) queue_full: u64,
    pub(crate) queue_closed: u64,
}

impl AlertDeliveryMetrics {
    pub(crate) fn snapshot(&self) -> AlertDeliverySnapshot {
        AlertDeliverySnapshot {
            outstanding: self.outstanding.load(Ordering::Relaxed),
            active: self.active.load(Ordering::Relaxed),
            queue_full: self.queue_full.load(Ordering::Relaxed),
            queue_closed: self.queue_closed.load(Ordering::Relaxed),
        }
    }

    pub(super) fn track_outstanding(&self) -> DeliveryGuard {
        DeliveryGuard::new(&self.outstanding)
    }

    pub(super) fn track_active(&self) -> DeliveryGuard {
        DeliveryGuard::new(&self.active)
    }

    pub(super) fn record_full(&self) {
        self.queue_full.fetch_add(1, Ordering::Relaxed);
    }

    pub(super) fn record_closed(&self) {
        self.queue_closed.fetch_add(1, Ordering::Relaxed);
    }
}

#[derive(Debug)]
pub(super) struct DeliveryGuard(Arc<AtomicU64>);

impl DeliveryGuard {
    fn new(counter: &Arc<AtomicU64>) -> Self {
        counter.fetch_add(1, Ordering::Relaxed);
        Self(Arc::clone(counter))
    }
}

impl Drop for DeliveryGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Relaxed);
    }
}
