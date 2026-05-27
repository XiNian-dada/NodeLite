use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use nodelite_proto::AlertingConfig;
use tokio::sync::RwLock;
use tokio::task::JoinHandle;
use tokio::time::{MissedTickBehavior, interval};
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use crate::state::SharedState;

use super::{
    AlertEvent, AlertEventKind, AlertStateTracker, deliver_alert_event, evaluate_rules,
    smtp_endpoint_label, webhook_endpoint_label,
};

const ALERT_EVALUATION_INTERVAL_SECS: u64 = 30;

pub(crate) fn spawn_alert_runtime(
    alerting: Arc<RwLock<AlertingConfig>>,
    shared: SharedState,
    shutdown: CancellationToken,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        run_alert_runtime(alerting, shared, shutdown).await;
    })
}

async fn run_alert_runtime(
    alerting: Arc<RwLock<AlertingConfig>>,
    shared: SharedState,
    shutdown: CancellationToken,
) {
    let mut tracker = AlertStateTracker::new();
    let mut ticker = interval(Duration::from_secs(ALERT_EVALUATION_INTERVAL_SECS));
    ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            _ = shutdown.cancelled() => break,
            _ = ticker.tick() => {
                let config = {
                    let alerting = alerting.read().await;
                    alerting.clone()
                };
                if !config.enabled || config.rules.is_empty() {
                    tracker.clear();
                    continue;
                }

                let now = Utc::now();
                let statuses = shared.list_statuses().await;
                let matches = evaluate_rules(&config.rules, &statuses, now);
                for event in tracker.update(&config.rules, &matches, now) {
                    log_alert_event(&event);
                    if let Err(error) = deliver_alert_event(&config, &event).await {
                        warn!(
                            error = ?error,
                            webhook = %webhook_endpoint_label(&config.webhook.url),
                            smtp = %smtp_endpoint_label(&config.smtp),
                            rule_id = %event.rule.id,
                            node_id = %event.node_id,
                            "failed to deliver alert notification",
                        );
                    }
                }
            }
        }
    }
}

fn log_alert_event(event: &AlertEvent) {
    let reading = event.reading.as_ref();
    info!(
        kind = alert_event_kind(event.kind),
        rule_id = %event.rule.id,
        rule_name = %event.rule.name,
        severity = ?event.rule.severity,
        node_id = %event.node_id,
        node_label = %event.node_label,
        occurred_at = %event.occurred_at,
        metric = ?reading.map(|reading| &reading.metric),
        value = reading.map(|reading| reading.value),
        threshold = reading.map(|reading| reading.threshold),
        "alert rule event evaluated",
    );
}

fn alert_event_kind(kind: AlertEventKind) -> &'static str {
    match kind {
        AlertEventKind::Triggered => "triggered",
        AlertEventKind::Resolved => "resolved",
    }
}
