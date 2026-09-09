//! Bound spawned tasks and result backlog as well as the input channel.

use std::future::Future;

use tokio::sync::mpsc;
use tokio::task::{JoinHandle, JoinSet};
use tracing::warn;

use crate::alerts::delivery_metrics::{
    AlertDeliveryMetrics, DeliveryGuard, MAX_CONCURRENT_DELIVERIES,
};
use crate::alerts::{InspectionSummary, deliver_alert_event, deliver_inspection_summary};
use crate::queue::{QueueSendError, bounded_mpsc_channel};

use super::{DeliveryJob, DeliveryResult};

pub(super) type QueuedDelivery = (DeliveryJob, DeliveryGuard);
pub(super) type CompletedDelivery = (DeliveryResult, DeliveryGuard);

pub(super) struct DeliverySender {
    sender: mpsc::Sender<QueuedDelivery>,
    metrics: AlertDeliveryMetrics,
}

pub(super) fn delivery_channel(
    capacity: usize,
    metrics: AlertDeliveryMetrics,
) -> (DeliverySender, mpsc::Receiver<QueuedDelivery>) {
    let (sender, receiver) = bounded_mpsc_channel(capacity);
    (DeliverySender { sender, metrics }, receiver)
}

impl DeliverySender {
    pub(super) fn try_send(&self, job: DeliveryJob) -> Result<(), QueueSendError> {
        // Reserve first so a rejected item never briefly exceeds the outstanding-work budget.
        match self.sender.try_reserve() {
            Ok(slot) => {
                slot.send((job, self.metrics.track_outstanding()));
                Ok(())
            }
            Err(mpsc::error::TrySendError::Full(_)) => {
                self.metrics.record_full();
                Err(QueueSendError::Full)
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                self.metrics.record_closed();
                Err(QueueSendError::Closed)
            }
        }
    }
}

pub(super) fn spawn_delivery_dispatcher(
    delivery_rx: mpsc::Receiver<QueuedDelivery>,
    result_tx: mpsc::Sender<CompletedDelivery>,
    metrics: AlertDeliveryMetrics,
) -> JoinHandle<()> {
    spawn_dispatcher_with(delivery_rx, result_tx, metrics, deliver_job)
}

fn spawn_dispatcher_with<F, Fut>(
    mut delivery_rx: mpsc::Receiver<QueuedDelivery>,
    result_tx: mpsc::Sender<CompletedDelivery>,
    metrics: AlertDeliveryMetrics,
    deliver: F,
) -> JoinHandle<()>
where
    F: Fn(DeliveryJob) -> Fut + Clone + Send + 'static,
    Fut: Future<Output = DeliveryResult> + Send + 'static,
{
    tokio::spawn(async move {
        let mut deliveries = JoinSet::new();
        loop {
            tokio::select! {
                biased;
                Some(result) = deliveries.join_next(), if !deliveries.is_empty() => {
                    if let Err(error) = result {
                        warn!(error = ?error, "alert delivery task join failed");
                    }
                }
                Some((job, outstanding)) = delivery_rx.recv(),
                    if deliveries.len() < MAX_CONCURRENT_DELIVERIES => {
                    let result_tx = result_tx.clone();
                    let active = metrics.track_active();
                    let deliver = deliver.clone();
                    deliveries.spawn(async move {
                        let _active = active;
                        let result = deliver(job).await;
                        // A slow result consumer also retains a worker slot instead of spawning more work.
                        let _ = result_tx.send((result, outstanding)).await;
                    });
                }
                else => break,
            }
        }
    })
}

async fn deliver_job(job: DeliveryJob) -> DeliveryResult {
    match job {
        DeliveryJob::Alert { config, event } => {
            let result = deliver_alert_event(&config, &event).await;
            DeliveryResult::Alert {
                config,
                event,
                result,
            }
        }
        DeliveryJob::Inspection {
            config,
            occurred_at,
            local_date,
            lookback_hours,
            report,
            trends,
        } => {
            let summary = InspectionSummary {
                occurred_at,
                local_date,
                lookback_hours,
                report: &report,
                trends: &trends,
            };
            let result = deliver_inspection_summary(&config, &summary).await;
            DeliveryResult::Inspection {
                config,
                local_date,
                report,
                result,
            }
        }
    }
}

#[cfg(test)]
mod tests;
