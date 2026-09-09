//! Shutdown has a finite drain window even when external notification endpoints stop responding.

use std::time::Duration;

use tokio::task::JoinHandle;
use tokio::time::timeout;
use tokio_util::task::AbortOnDropHandle;
use tracing::{info, warn};

const DELIVERY_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DeliveryDrainOutcome {
    Drained,
    JoinFailed,
    TimedOut,
}

pub(super) async fn drain_delivery_dispatcher(
    delivery_dispatcher: JoinHandle<()>,
) -> DeliveryDrainOutcome {
    drain_delivery_dispatcher_with_timeout(delivery_dispatcher, DELIVERY_SHUTDOWN_TIMEOUT).await
}

pub(super) async fn drain_delivery_dispatcher_with_timeout(
    delivery_dispatcher: JoinHandle<()>,
    timeout_duration: Duration,
) -> DeliveryDrainOutcome {
    // The server's shared deadline can cancel this drain before its own timeout.
    let mut delivery_dispatcher = AbortOnDropHandle::new(delivery_dispatcher);
    match timeout(timeout_duration, &mut delivery_dispatcher).await {
        Ok(Ok(())) => {
            info!("alert delivery dispatcher drained during shutdown");
            DeliveryDrainOutcome::Drained
        }
        Ok(Err(error)) => {
            warn!(error = ?error, "alert delivery dispatcher failed during shutdown");
            DeliveryDrainOutcome::JoinFailed
        }
        Err(_) => {
            delivery_dispatcher.abort();
            let _ = delivery_dispatcher.await;
            warn!(
                timeout_secs = timeout_duration.as_secs(),
                "alert delivery dispatcher did not drain before shutdown timeout"
            );
            DeliveryDrainOutcome::TimedOut
        }
    }
}

#[cfg(test)]
mod tests {
    use std::future::pending;
    use std::time::Duration;

    use tokio::sync::oneshot;

    use super::drain_delivery_dispatcher_with_timeout;

    #[tokio::test(start_paused = true)]
    async fn cancelling_drain_also_aborts_the_dispatcher() {
        let (sender, receiver) = oneshot::channel::<()>();
        let dispatcher = tokio::spawn(async move {
            let _sender = sender;
            pending::<()>().await;
        });
        let mut drain = Box::pin(drain_delivery_dispatcher_with_timeout(
            dispatcher,
            Duration::from_secs(60),
        ));
        assert!(futures::poll!(drain.as_mut()).is_pending());

        drop(drain);

        assert!(
            tokio::time::timeout(Duration::from_secs(1), receiver)
                .await
                .expect("cancelled dispatcher should be dropped")
                .is_err()
        );
    }
}
