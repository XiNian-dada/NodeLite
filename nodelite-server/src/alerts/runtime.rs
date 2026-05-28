use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Local, NaiveDate, NaiveTime, Utc};
use nodelite_proto::{AlertChannel, AlertingConfig};
use tokio::sync::{RwLock, Semaphore, mpsc};
use tokio::task::JoinHandle;
use tokio::time::{MissedTickBehavior, interval};
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use crate::state::SharedState;

use super::delivery::AlertDeliveryError;
use super::{
    AlertEvent, AlertEventKind, AlertStateTracker, InspectionReport, InspectionSummary,
    build_inspection_report, deliver_alert_event, deliver_inspection_summary, evaluate_rules,
    smtp_endpoint_label, webhook_endpoint_label,
};

const ALERT_EVALUATION_INTERVAL_SECS: u64 = 30;
const INSPECTION_RETRY_INTERVAL_SECS: i64 = 300;
const DELIVERY_QUEUE_CAPACITY: usize = 1024;
const MAX_CONCURRENT_DELIVERIES: usize = 8;

#[derive(Debug)]
enum DeliveryJob {
    Alert {
        config: AlertingConfig,
        event: AlertEvent,
    },
    Inspection {
        config: AlertingConfig,
        occurred_at: DateTime<Utc>,
        local_date: NaiveDate,
        lookback_hours: u64,
        report: InspectionReport,
    },
}

#[derive(Debug)]
enum DeliveryResult {
    Alert {
        config: AlertingConfig,
        event: AlertEvent,
        result: Result<(), AlertDeliveryError>,
    },
    Inspection {
        config: AlertingConfig,
        local_date: NaiveDate,
        report: InspectionReport,
        result: Result<(), AlertDeliveryError>,
    },
}

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
    let mut inspection_dispatch = InspectionDispatchState::new();
    let (delivery_tx, delivery_rx) = mpsc::channel(DELIVERY_QUEUE_CAPACITY);
    let (result_tx, mut result_rx) = mpsc::unbounded_channel();
    let delivery_dispatcher = spawn_delivery_dispatcher(delivery_rx, result_tx);
    let mut ticker = interval(Duration::from_secs(ALERT_EVALUATION_INTERVAL_SECS));
    ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            _ = shutdown.cancelled() => break,
            _ = ticker.tick() => {
                process_delivery_results(
                    &mut result_rx,
                    &mut tracker,
                    &mut inspection_dispatch,
                    &delivery_tx,
                );
                let config = {
                    let alerting = alerting.read().await;
                    alerting.clone()
                };
                if !config.enabled {
                    tracker.clear();
                    inspection_dispatch.clear();
                    continue;
                }

                let now = Utc::now();
                let statuses = shared.list_statuses().await;
                if config.rules.is_empty() {
                    tracker.clear();
                } else {
                    let matches = evaluate_rules(&config.rules, &statuses, now);
                    for event in tracker.update(&config.rules, &matches, now) {
                        log_alert_event(&event);
                        enqueue_alert_delivery(&delivery_tx, &mut tracker, &config, &event, now);
                    }
                }

                if should_check_inspection(&config)
                    && let Some(local_date) =
                        inspection_dispatch.due_date(&config.inspection.local_time, Local::now(), now)
                {
                    let report = build_inspection_report(&config.inspection, &statuses, now);
                    enqueue_inspection_delivery(
                        &delivery_tx,
                        &mut inspection_dispatch,
                        &config,
                        report,
                        local_date,
                        now,
                    );
                }
            }
        }
    }
    drop(delivery_tx);
    delivery_dispatcher.abort();
}

fn spawn_delivery_dispatcher(
    mut delivery_rx: mpsc::Receiver<DeliveryJob>,
    result_tx: mpsc::UnboundedSender<DeliveryResult>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let limiter = Arc::new(Semaphore::new(MAX_CONCURRENT_DELIVERIES));
        while let Some(job) = delivery_rx.recv().await {
            let limiter = Arc::clone(&limiter);
            let result_tx = result_tx.clone();
            tokio::spawn(async move {
                let Ok(_permit) = limiter.acquire_owned().await else {
                    return;
                };
                let result = deliver_job(job).await;
                let _ = result_tx.send(result);
            });
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
        } => {
            let summary = InspectionSummary {
                occurred_at,
                local_date,
                lookback_hours,
                report: &report,
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

fn process_delivery_results(
    result_rx: &mut mpsc::UnboundedReceiver<DeliveryResult>,
    tracker: &mut AlertStateTracker,
    inspection_dispatch: &mut InspectionDispatchState,
    delivery_tx: &mpsc::Sender<DeliveryJob>,
) {
    while let Ok(result) = result_rx.try_recv() {
        match result {
            DeliveryResult::Alert {
                config,
                event,
                result,
            } => handle_alert_delivery_result(tracker, delivery_tx, &config, &event, result),
            DeliveryResult::Inspection {
                config,
                local_date,
                report,
                result,
            } => handle_inspection_delivery_result(
                inspection_dispatch,
                &config,
                local_date,
                &report,
                result,
            ),
        }
    }
}

fn enqueue_alert_delivery(
    delivery_tx: &mpsc::Sender<DeliveryJob>,
    tracker: &mut AlertStateTracker,
    config: &AlertingConfig,
    event: &AlertEvent,
    now: DateTime<Utc>,
) {
    if delivery_tx
        .try_send(DeliveryJob::Alert {
            config: config.clone(),
            event: event.clone(),
        })
        .is_err()
    {
        tracker.record_delivery_failure(event, now);
        warn!(
            webhook = %webhook_endpoint_label(&config.webhook.url),
            smtp = %smtp_endpoint_label(&config.smtp),
            rule_id = %event.rule.id,
            node_id = %event.node_id,
            "failed to enqueue alert notification delivery",
        );
    }
}

fn enqueue_inspection_delivery(
    delivery_tx: &mpsc::Sender<DeliveryJob>,
    inspection_dispatch: &mut InspectionDispatchState,
    config: &AlertingConfig,
    report: InspectionReport,
    local_date: NaiveDate,
    now: DateTime<Utc>,
) {
    if delivery_tx
        .try_send(DeliveryJob::Inspection {
            config: config.clone(),
            occurred_at: now,
            local_date,
            lookback_hours: config.inspection.lookback_hours,
            report,
        })
        .is_ok()
    {
        inspection_dispatch.mark_pending(local_date);
        return;
    }

    inspection_dispatch.mark_failed(now);
    warn!(
        webhook = %webhook_endpoint_label(&config.webhook.url),
        smtp = %smtp_endpoint_label(&config.smtp),
        local_date = %local_date,
        "failed to enqueue daily inspection summary delivery",
    );
}

fn handle_alert_delivery_result(
    tracker: &mut AlertStateTracker,
    delivery_tx: &mpsc::Sender<DeliveryJob>,
    config: &AlertingConfig,
    event: &AlertEvent,
    result: Result<(), AlertDeliveryError>,
) {
    match result {
        Ok(()) => {
            if let Some(resolved) = tracker.record_delivery_success(event) {
                log_alert_event(&resolved);
                enqueue_alert_delivery(delivery_tx, tracker, config, &resolved, Utc::now());
            }
        }
        Err(error) => {
            tracker.record_delivery_failure(event, Utc::now());
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

fn handle_inspection_delivery_result(
    inspection_dispatch: &mut InspectionDispatchState,
    config: &AlertingConfig,
    local_date: NaiveDate,
    report: &InspectionReport,
    result: Result<(), AlertDeliveryError>,
) {
    match result {
        Ok(()) => {
            inspection_dispatch.mark_sent(local_date);
            info!(
                local_date = %local_date,
                total_nodes = report.total_nodes,
                offline_nodes = report.offline_nodes,
                latency_nodes = report.latency_nodes,
                cpu_hot_nodes = report.cpu_hot_nodes,
                memory_hot_nodes = report.memory_hot_nodes,
                "daily inspection summary delivered",
            );
        }
        Err(error) => {
            inspection_dispatch.mark_failed(Utc::now());
            warn!(
                error = ?error,
                webhook = %webhook_endpoint_label(&config.webhook.url),
                smtp = %smtp_endpoint_label(&config.smtp),
                local_date = %local_date,
                "failed to deliver daily inspection summary",
            );
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

#[derive(Debug, Default)]
struct InspectionDispatchState {
    last_sent_date: Option<NaiveDate>,
    pending_date: Option<NaiveDate>,
    last_failed_at: Option<DateTime<Utc>>,
}

impl InspectionDispatchState {
    fn new() -> Self {
        Self::default()
    }

    fn clear(&mut self) {
        self.last_sent_date = None;
        self.pending_date = None;
        self.last_failed_at = None;
    }

    fn due_date(
        &self,
        configured_time: &str,
        local_now: DateTime<Local>,
        now: DateTime<Utc>,
    ) -> Option<NaiveDate> {
        let scheduled_time = parse_inspection_local_time(configured_time)?;
        self.due_date_for(
            local_now.date_naive(),
            local_now.time(),
            scheduled_time,
            now,
        )
    }

    fn due_date_for(
        &self,
        local_date: NaiveDate,
        local_time: NaiveTime,
        scheduled_time: NaiveTime,
        now: DateTime<Utc>,
    ) -> Option<NaiveDate> {
        if self.last_sent_date == Some(local_date)
            || self.pending_date == Some(local_date)
            || local_time < scheduled_time
        {
            return None;
        }
        if self.last_failed_at.is_some_and(|last_failed_at| {
            now.signed_duration_since(last_failed_at)
                < chrono::Duration::seconds(INSPECTION_RETRY_INTERVAL_SECS)
        }) {
            return None;
        }
        Some(local_date)
    }

    fn mark_sent(&mut self, local_date: NaiveDate) {
        self.last_sent_date = Some(local_date);
        self.pending_date = None;
        self.last_failed_at = None;
    }

    fn mark_pending(&mut self, local_date: NaiveDate) {
        self.pending_date = Some(local_date);
    }

    fn mark_failed(&mut self, now: DateTime<Utc>) {
        self.pending_date = None;
        self.last_failed_at = Some(now);
    }
}

fn should_check_inspection(config: &AlertingConfig) -> bool {
    if !config.inspection.enabled {
        return false;
    }
    let smtp_enabled =
        config.smtp.enabled && config.inspection.delivery.contains(&AlertChannel::Smtp);
    let webhook_enabled =
        config.webhook.enabled && config.inspection.delivery.contains(&AlertChannel::Webhook);
    smtp_enabled || webhook_enabled
}

fn parse_inspection_local_time(value: &str) -> Option<NaiveTime> {
    let mut parts = value.trim().split(':');
    let (Some(hours), Some(minutes), None) = (parts.next(), parts.next(), parts.next()) else {
        return None;
    };
    NaiveTime::from_hms_opt(hours.parse::<u32>().ok()?, minutes.parse::<u32>().ok()?, 0)
}

#[cfg(test)]
mod tests {
    use chrono::{Duration, NaiveDate, NaiveTime, Utc};
    use nodelite_proto::{AlertChannel, AlertingConfig};

    use super::{InspectionDispatchState, parse_inspection_local_time, should_check_inspection};

    #[test]
    fn inspection_dispatch_waits_until_configured_time() {
        let state = InspectionDispatchState::new();
        let date = NaiveDate::from_ymd_opt(2026, 5, 27).expect("date should be valid");
        let scheduled = NaiveTime::from_hms_opt(9, 0, 0).expect("time should be valid");

        assert!(
            state
                .due_date_for(
                    date,
                    NaiveTime::from_hms_opt(8, 59, 0).expect("time should be valid"),
                    scheduled,
                    Utc::now(),
                )
                .is_none()
        );
        assert_eq!(
            state.due_date_for(date, scheduled, scheduled, Utc::now()),
            Some(date)
        );
    }

    #[test]
    fn inspection_dispatch_sends_once_per_local_date() {
        let mut state = InspectionDispatchState::new();
        let date = NaiveDate::from_ymd_opt(2026, 5, 27).expect("date should be valid");
        let time = NaiveTime::from_hms_opt(9, 0, 0).expect("time should be valid");

        state.mark_sent(date);

        assert!(state.due_date_for(date, time, time, Utc::now()).is_none());
        assert_eq!(
            state.due_date_for(
                date.succ_opt().expect("next day should exist"),
                time,
                time,
                Utc::now()
            ),
            Some(date.succ_opt().expect("next day should exist"))
        );
    }

    #[test]
    fn inspection_dispatch_delays_retry_after_failure() {
        let mut state = InspectionDispatchState::new();
        let date = NaiveDate::from_ymd_opt(2026, 5, 27).expect("date should be valid");
        let time = NaiveTime::from_hms_opt(9, 0, 0).expect("time should be valid");
        let now = Utc::now();
        state.mark_failed(now);

        assert!(
            state
                .due_date_for(date, time, time, now + Duration::minutes(1))
                .is_none()
        );
        assert_eq!(
            state.due_date_for(date, time, time, now + Duration::minutes(6)),
            Some(date)
        );
    }

    #[test]
    fn inspection_dispatch_suppresses_duplicate_while_pending() {
        let mut state = InspectionDispatchState::new();
        let date = NaiveDate::from_ymd_opt(2026, 5, 27).expect("date should be valid");
        let time = NaiveTime::from_hms_opt(9, 0, 0).expect("time should be valid");

        state.mark_pending(date);

        assert!(state.due_date_for(date, time, time, Utc::now()).is_none());
    }

    #[test]
    fn parse_inspection_time_accepts_valid_hh_mm() {
        assert_eq!(
            parse_inspection_local_time("09:30"),
            NaiveTime::from_hms_opt(9, 30, 0)
        );
        assert!(parse_inspection_local_time("24:61").is_none());
    }

    #[test]
    fn inspection_requires_enabled_delivery_channel() {
        let mut config = AlertingConfig {
            enabled: true,
            inspection: nodelite_proto::InspectionConfig {
                enabled: true,
                delivery: vec![AlertChannel::Webhook],
                ..nodelite_proto::InspectionConfig::default()
            },
            ..AlertingConfig::default()
        };

        assert!(!should_check_inspection(&config));
        config.webhook.enabled = true;
        assert!(should_check_inspection(&config));
    }
}
