//! Bounded queue helpers shared by non-blocking server writers.
//!
//! These helpers keep real-time paths from being backpressured by slower background tasks,
//! while still preserving the caller-specific policy for full and closed queues.

use std::sync::atomic::{AtomicU64, Ordering};

use tokio::sync::mpsc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum QueueSendError {
    Full,
    Closed,
}

pub(crate) fn bounded_mpsc_channel<T>(capacity: usize) -> (mpsc::Sender<T>, mpsc::Receiver<T>) {
    mpsc::channel(capacity)
}

pub(crate) fn try_enqueue<T>(tx: &mpsc::Sender<T>, item: T) -> Result<(), QueueSendError> {
    tx.try_send(item).map_err(|error| match error {
        mpsc::error::TrySendError::Full(_) => QueueSendError::Full,
        mpsc::error::TrySendError::Closed(_) => QueueSendError::Closed,
    })
}

pub(crate) fn record_dropped_write(counter: &AtomicU64) -> u64 {
    counter.fetch_add(1, Ordering::Relaxed) + 1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn try_enqueue_reports_full_without_waiting() {
        let (tx, _rx) = bounded_mpsc_channel(1);

        try_enqueue(&tx, 1).expect("first item should fit");
        let error = try_enqueue(&tx, 2).expect_err("full queue should fail immediately");

        assert_eq!(error, QueueSendError::Full);
    }

    #[test]
    fn try_enqueue_reports_closed_sender() {
        let (tx, rx) = bounded_mpsc_channel::<u8>(1);
        drop(rx);

        let error = try_enqueue(&tx, 1).expect_err("closed queue should fail immediately");

        assert_eq!(error, QueueSendError::Closed);
    }

    #[test]
    fn record_dropped_write_returns_updated_total() {
        let counter = AtomicU64::new(3);

        let total = record_dropped_write(&counter);

        assert_eq!(total, 4);
        assert_eq!(counter.load(Ordering::Relaxed), 4);
    }
}
