//! Test-only instrumentation for concurrent history query execution.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

pub(super) struct HistoryQueryProbe {
    active: AtomicUsize,
    max_active: AtomicUsize,
    total_entered: AtomicUsize,
    hold_for: Duration,
}

impl HistoryQueryProbe {
    pub(super) fn new(hold_for: Duration) -> Self {
        Self {
            active: AtomicUsize::new(0),
            max_active: AtomicUsize::new(0),
            total_entered: AtomicUsize::new(0),
            hold_for,
        }
    }

    pub(super) fn enter(self: &Arc<Self>) -> HistoryQueryProbeGuard {
        self.total_entered.fetch_add(1, Ordering::Relaxed);
        let active = self.active.fetch_add(1, Ordering::Relaxed) + 1;
        self.max_active.fetch_max(active, Ordering::Relaxed);
        if !self.hold_for.is_zero() {
            std::thread::sleep(self.hold_for);
        }
        HistoryQueryProbeGuard {
            probe: Arc::clone(self),
        }
    }

    pub(super) fn max_active(&self) -> usize {
        self.max_active.load(Ordering::Relaxed)
    }

    pub(super) fn total_entered(&self) -> usize {
        self.total_entered.load(Ordering::Relaxed)
    }
}

pub(super) struct HistoryQueryProbeGuard {
    probe: Arc<HistoryQueryProbe>,
}

impl Drop for HistoryQueryProbeGuard {
    fn drop(&mut self) {
        self.probe.active.fetch_sub(1, Ordering::Relaxed);
    }
}
