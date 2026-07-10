//! Prometheus rendering for Argon2 token verification pressure.

use crate::registry::TokenVerifyMetrics;

use super::MetricEmitter;

pub(crate) fn render_token_verify_metrics(metrics: TokenVerifyMetrics) -> String {
    let mut emitter = MetricEmitter::default();
    emitter.gauge(
        "nodelite_token_verify_limit",
        "Configured maximum number of concurrent Argon2 token verifications.",
        &[],
        metrics.limit,
    );
    emitter.gauge(
        "nodelite_token_verify_active",
        "Number of Argon2 token verifications currently executing.",
        &[],
        metrics.active,
    );
    emitter.gauge(
        "nodelite_token_verify_waiting",
        "Number of token verification requests waiting for the concurrency limiter.",
        &[],
        metrics.waiting,
    );
    emitter.counter(
        "nodelite_token_verify_wait_seconds_total",
        "Cumulative time token verification requests spent waiting for the concurrency limiter.",
        &[],
        metrics.wait_seconds_total,
    );
    emitter.finish()
}
