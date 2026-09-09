//! Saturate real dispatcher channels with deterministic stalled delivery futures.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use chrono::Utc;
use tokio::sync::Notify;
use tokio::time::{sleep, timeout};

use crate::alerts::AlertStateTracker;
use crate::alerts::delivery_metrics::{
    DELIVERY_QUEUE_CAPACITY, DELIVERY_RESULT_CAPACITY, DELIVERY_TOTAL_CAPACITY,
};

use super::super::drain::{DeliveryDrainOutcome, drain_delivery_dispatcher_with_timeout};
use super::super::tests::{alerting_config, matched, rule};
use super::*;

fn job() -> DeliveryJob {
    let event = AlertStateTracker::new()
        .update(&[rule()], &[matched(91)], Utc::now())
        .pop()
        .expect("matched rule triggers an event");
    DeliveryJob::Alert {
        config: alerting_config(),
        event,
    }
}

fn succeeded(job: DeliveryJob) -> DeliveryResult {
    match job {
        DeliveryJob::Alert { config, event } => DeliveryResult::Alert {
            config,
            event,
            result: Ok(()),
        },
        DeliveryJob::Inspection { .. } => panic!("test queues only alert jobs"),
    }
}

async fn wait_for(mut condition: impl FnMut() -> bool) {
    timeout(Duration::from_secs(2), async {
        while !condition() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("dispatcher should make progress");
}

#[tokio::test]
async fn stalled_deliveries_cannot_escape_the_queue_and_worker_budget() {
    let metrics = AlertDeliveryMetrics::default();
    let (tx, rx) = delivery_channel(DELIVERY_QUEUE_CAPACITY, metrics.clone());
    let (results, result_rx) = mpsc::channel(DELIVERY_RESULT_CAPACITY);
    let started = Arc::new(AtomicUsize::new(0));
    let gate = Arc::new(Notify::new());
    let dispatcher = spawn_dispatcher_with(rx, results, metrics.clone(), {
        let started = Arc::clone(&started);
        move |job| {
            let started = Arc::clone(&started);
            let gate = Arc::clone(&gate);
            async move {
                started.fetch_add(1, Ordering::Relaxed);
                gate.notified().await;
                succeeded(job)
            }
        }
    });
    for _ in 0..MAX_CONCURRENT_DELIVERIES {
        tx.try_send(job()).expect("initial worker job");
    }
    wait_for(|| started.load(Ordering::Relaxed) == MAX_CONCURRENT_DELIVERIES).await;
    for _ in 0..DELIVERY_QUEUE_CAPACITY {
        tx.try_send(job()).expect("bounded waiting slot");
    }
    for _ in 0..DELIVERY_QUEUE_CAPACITY * 3 {
        assert_eq!(tx.try_send(job()), Err(QueueSendError::Full));
    }
    sleep(Duration::from_millis(25)).await;
    let snapshot = metrics.snapshot();
    assert_eq!(started.load(Ordering::Relaxed), MAX_CONCURRENT_DELIVERIES);
    assert_eq!(snapshot.active, MAX_CONCURRENT_DELIVERIES as u64);
    assert_eq!(
        snapshot.outstanding,
        (DELIVERY_QUEUE_CAPACITY + MAX_CONCURRENT_DELIVERIES) as u64
    );
    assert_eq!(snapshot.queue_full, (DELIVERY_QUEUE_CAPACITY * 3) as u64);

    drop(tx);
    drop(result_rx);
    let outcome =
        drain_delivery_dispatcher_with_timeout(dispatcher, Duration::from_millis(10)).await;
    assert_eq!(outcome, DeliveryDrainOutcome::TimedOut);
    wait_for(|| metrics.snapshot().outstanding == 0 && metrics.snapshot().active == 0).await;
}

async fn saturated_results() -> (
    DeliverySender,
    mpsc::Receiver<CompletedDelivery>,
    JoinHandle<()>,
    AlertDeliveryMetrics,
) {
    let metrics = AlertDeliveryMetrics::default();
    let (tx, rx) = delivery_channel(DELIVERY_QUEUE_CAPACITY, metrics.clone());
    let (result_tx, result_rx) = mpsc::channel(DELIVERY_RESULT_CAPACITY);
    let dispatcher = spawn_dispatcher_with(rx, result_tx, metrics.clone(), |job| {
        std::future::ready(succeeded(job))
    });
    for _ in 0..DELIVERY_RESULT_CAPACITY {
        tx.try_send(job()).expect("buffered result");
    }
    wait_for(|| result_rx.len() == DELIVERY_RESULT_CAPACITY && metrics.snapshot().active == 0)
        .await;
    for _ in 0..MAX_CONCURRENT_DELIVERIES {
        tx.try_send(job()).expect("blocked result sender");
    }
    wait_for(|| {
        tx.sender.capacity() == DELIVERY_QUEUE_CAPACITY
            && metrics.snapshot().active == MAX_CONCURRENT_DELIVERIES as u64
    })
    .await;
    for _ in 0..DELIVERY_QUEUE_CAPACITY {
        tx.try_send(job()).expect("waiting job");
    }
    assert_eq!(tx.try_send(job()), Err(QueueSendError::Full));
    assert_eq!(
        metrics.snapshot().outstanding,
        DELIVERY_TOTAL_CAPACITY as u64
    );
    (tx, result_rx, dispatcher, metrics)
}

#[tokio::test]
async fn completed_results_are_bounded_and_consumption_releases_every_job() {
    let (tx, mut results, dispatcher, metrics) = saturated_results().await;
    drop(tx);
    let delivered = timeout(Duration::from_secs(3), async {
        let mut delivered = 0;
        while let Some((result, _outstanding)) = results.recv().await {
            assert!(matches!(
                result,
                DeliveryResult::Alert { result: Ok(()), .. }
            ));
            delivered += 1;
            assert!(metrics.snapshot().outstanding <= DELIVERY_TOTAL_CAPACITY as u64);
        }
        delivered
    })
    .await
    .expect("result consumption must unblock workers");
    assert_eq!(delivered, DELIVERY_TOTAL_CAPACITY);
    assert_eq!(
        drain_delivery_dispatcher_with_timeout(dispatcher, Duration::from_secs(1)).await,
        DeliveryDrainOutcome::Drained
    );
    assert_eq!(metrics.snapshot().outstanding, 0);
    assert_eq!(metrics.snapshot().active, 0);
}

#[tokio::test]
async fn shutdown_drains_even_if_the_result_queue_was_full() {
    let (tx, results, dispatcher, metrics) = saturated_results().await;
    drop(tx);
    drop(results);
    assert_eq!(
        drain_delivery_dispatcher_with_timeout(dispatcher, Duration::from_secs(2)).await,
        DeliveryDrainOutcome::Drained
    );
    assert_eq!(metrics.snapshot().outstanding, 0);
    assert_eq!(metrics.snapshot().active, 0);
}

#[test]
fn a_closed_queue_rejects_and_counts_jobs_without_leaking_the_budget() {
    let metrics = AlertDeliveryMetrics::default();
    let (tx, rx) = delivery_channel(1, metrics.clone());
    drop(rx);
    assert_eq!(tx.try_send(job()), Err(QueueSendError::Closed));
    assert_eq!(metrics.snapshot().queue_closed, 1);
    assert_eq!(metrics.snapshot().outstanding, 0);
}
