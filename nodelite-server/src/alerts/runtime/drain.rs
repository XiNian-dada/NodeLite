use std::time::Duration;

use tokio::task::JoinHandle;
use tokio::time::timeout;
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
    mut delivery_dispatcher: JoinHandle<()>,
    timeout_duration: Duration,
) -> DeliveryDrainOutcome {
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
            warn!(
                timeout_secs = timeout_duration.as_secs(),
                "alert delivery dispatcher did not drain before shutdown timeout"
            );
            DeliveryDrainOutcome::TimedOut
        }
    }
}
