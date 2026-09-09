//! Include pending acknowledgements in the alert delivery resource budget.

use crate::alerts::{
    AlertDeliverySnapshot, DELIVERY_QUEUE_CAPACITY, DELIVERY_RESULT_CAPACITY,
    DELIVERY_TOTAL_CAPACITY, MAX_CONCURRENT_DELIVERIES,
};

use super::emitter::MetricEmitter;

pub(super) fn render_alert_delivery_metrics(
    emitter: &mut MetricEmitter,
    metrics: AlertDeliverySnapshot,
) {
    for (name, help, value) in [
        (
            "nodelite_alert_delivery_outstanding",
            "Accepted deliveries still queued, active or awaiting result consumption.",
            metrics.outstanding,
        ),
        (
            "nodelite_alert_delivery_active",
            "Delivery workers running or waiting to publish a result.",
            metrics.active,
        ),
        (
            "nodelite_alert_delivery_capacity",
            "Maximum accepted deliveries across queue, workers and result backlog.",
            DELIVERY_TOTAL_CAPACITY as u64,
        ),
        (
            "nodelite_alert_delivery_queue_capacity",
            "Maximum deliveries waiting for a worker.",
            DELIVERY_QUEUE_CAPACITY as u64,
        ),
        (
            "nodelite_alert_delivery_concurrency_limit",
            "Maximum spawned delivery workers, including blocked result sends.",
            MAX_CONCURRENT_DELIVERIES as u64,
        ),
        (
            "nodelite_alert_delivery_result_capacity",
            "Maximum buffered delivery results.",
            DELIVERY_RESULT_CAPACITY as u64,
        ),
    ] {
        emitter.gauge(name, help, &[], value);
    }
    emitter.counter(
        "nodelite_alert_delivery_queue_full_total",
        "Deliveries rejected because the input queue was full; eligible for tracker retry.",
        &[],
        metrics.queue_full,
    );
    emitter.counter(
        "nodelite_alert_delivery_queue_closed_total",
        "Deliveries rejected because the dispatcher input queue was closed.",
        &[],
        metrics.queue_closed,
    );
}
