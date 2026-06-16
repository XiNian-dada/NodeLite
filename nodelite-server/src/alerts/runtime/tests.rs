use std::sync::Arc;

use chrono::{Duration, NaiveDate, NaiveTime, Utc};
use nodelite_proto::{
    AlertChannel, AlertComparator, AlertMetric, AlertRuleConfig, AlertScopeMode, AlertSeverity,
    AlertingConfig,
};
use tokio::sync::mpsc;

use super::{
    DeliveryJob, DeliveryResult, InspectionDispatchState, enqueue_alert_delivery,
    enqueue_inspection_delivery, handle_alert_delivery_result, parse_inspection_local_time,
    process_delivery_results, should_check_inspection,
};
use crate::alerts::delivery::AlertDeliveryError;
use crate::alerts::{
    AlertEventKind, AlertMetricReading, AlertStateTracker, EvaluatedRule, InspectionReport,
};

fn rule() -> AlertRuleConfig {
    AlertRuleConfig {
        id: "cpu-hot".to_string(),
        name: "CPU".to_string(),
        enabled: true,
        metric: AlertMetric::CpuUsagePercent,
        comparator: AlertComparator::Gt,
        threshold: 90,
        window_minutes: 5,
        severity: AlertSeverity::Critical,
        scope_mode: AlertScopeMode::All,
        node_ids: Vec::new(),
        tags: Vec::new(),
        delivery: vec![AlertChannel::Webhook],
        cooldown_minutes: 30,
        send_resolved: true,
    }
}

fn matched(value: u64) -> EvaluatedRule {
    EvaluatedRule {
        rule_id: "cpu-hot".to_string(),
        node_id: "hk-01".to_string(),
        node_label: "Hong Kong".to_string(),
        reading: AlertMetricReading {
            metric: AlertMetric::CpuUsagePercent,
            value,
            threshold: 90,
        },
    }
}

fn report() -> InspectionReport {
    InspectionReport {
        total_nodes: 2,
        offline_nodes: 1,
        latency_nodes: 0,
        cpu_hot_nodes: 1,
        memory_hot_nodes: 0,
        highlights: Vec::new(),
    }
}

fn alerting_config() -> Arc<AlertingConfig> {
    Arc::new(AlertingConfig {
        enabled: true,
        webhook: nodelite_proto::AlertWebhookConfig {
            enabled: true,
            url: "https://alerts.example.test/hook".to_string(),
            secret: None,
            send_resolved: true,
        },
        ..AlertingConfig::default()
    })
}

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
fn enqueue_alert_delivery_records_failure_when_queue_is_full() {
    let now = Utc::now();
    let rules = vec![rule()];
    let config = alerting_config();
    let mut tracker = AlertStateTracker::new();
    let first = tracker.update(&rules, &[matched(91)], now);
    let event = first[0].clone();
    let (delivery_tx, _delivery_rx) = mpsc::channel(1);
    delivery_tx
        .try_send(DeliveryJob::Alert {
            config: Arc::clone(&config),
            event: event.clone(),
        })
        .expect("prefill should fit in queue");

    enqueue_alert_delivery(&delivery_tx, &mut tracker, &config, &event, now);
    let retry = tracker.update(&rules, &[matched(92)], now + Duration::minutes(5));

    assert_eq!(retry.len(), 1);
    assert_eq!(retry[0].kind, AlertEventKind::Triggered);
    assert_eq!(
        retry[0].reading.as_ref().map(|reading| reading.value),
        Some(92)
    );
}

#[test]
fn alert_delivery_failure_result_allows_retry() {
    let now = Utc::now();
    let rules = vec![rule()];
    let config = alerting_config();
    let mut tracker = AlertStateTracker::new();
    let first = tracker.update(&rules, &[matched(91)], now);
    let (delivery_tx, mut delivery_rx) = mpsc::channel(4);

    enqueue_alert_delivery(&delivery_tx, &mut tracker, &config, &first[0], now);
    let queued = delivery_rx
        .try_recv()
        .expect("triggered event should be queued before failure");
    match queued {
        DeliveryJob::Alert { event, .. } => {
            assert_eq!(event.kind, AlertEventKind::Triggered);
        }
        DeliveryJob::Inspection { .. } => panic!("expected alert delivery job"),
    }

    handle_alert_delivery_result(
        &mut tracker,
        &delivery_tx,
        &config,
        &first[0],
        Err(AlertDeliveryError::Timeout),
    );
    let retry = tracker.update(&rules, &[matched(93)], now + Duration::minutes(6));

    assert_eq!(retry.len(), 1);
    assert_eq!(retry[0].kind, AlertEventKind::Triggered);
    assert_eq!(
        retry[0].reading.as_ref().map(|reading| reading.value),
        Some(93)
    );
}

#[test]
fn process_delivery_results_enqueues_resolved_after_trigger_success() {
    let now = Utc::now();
    let rules = vec![rule()];
    let config = alerting_config();
    let mut tracker = AlertStateTracker::new();
    let mut inspection_dispatch = InspectionDispatchState::new();
    let first = tracker.update(&rules, &[matched(91)], now);
    let skipped = tracker.update(&rules, &[], now + Duration::minutes(1));
    let (delivery_tx, mut delivery_rx) = mpsc::channel(4);
    let (result_tx, mut result_rx) = mpsc::unbounded_channel();
    result_tx
        .send(DeliveryResult::Alert {
            config,
            event: first[0].clone(),
            result: Ok(()),
        })
        .expect("result receiver should be open");

    process_delivery_results(
        &mut result_rx,
        &mut tracker,
        &mut inspection_dispatch,
        &delivery_tx,
    );
    let queued = delivery_rx
        .try_recv()
        .expect("resolved event should be queued");

    assert!(skipped.is_empty());
    match queued {
        DeliveryJob::Alert { event, .. } => {
            assert_eq!(event.kind, AlertEventKind::Resolved);
            assert_eq!(event.occurred_at, now + Duration::minutes(1));
        }
        DeliveryJob::Inspection { .. } => panic!("expected alert delivery job"),
    }
}

#[test]
fn enqueue_inspection_delivery_marks_pending_and_queues_job() {
    let now = Utc::now();
    let date = NaiveDate::from_ymd_opt(2026, 5, 27).expect("date should be valid");
    let time = NaiveTime::from_hms_opt(9, 0, 0).expect("time should be valid");
    let config = alerting_config();
    let mut inspection_dispatch = InspectionDispatchState::new();
    let (delivery_tx, mut delivery_rx) = mpsc::channel(1);

    enqueue_inspection_delivery(
        &delivery_tx,
        &mut inspection_dispatch,
        &config,
        report(),
        date,
        now,
    );
    let queued = delivery_rx
        .try_recv()
        .expect("inspection job should be queued");

    assert!(
        inspection_dispatch
            .due_date_for(date, time, time, now)
            .is_none()
    );
    match queued {
        DeliveryJob::Inspection {
            local_date,
            lookback_hours,
            ..
        } => {
            assert_eq!(local_date, date);
            assert_eq!(lookback_hours, config.inspection.lookback_hours);
        }
        DeliveryJob::Alert { .. } => panic!("expected inspection delivery job"),
    }
}

#[test]
fn enqueue_inspection_delivery_marks_retry_when_queue_is_full() {
    let now = Utc::now();
    let date = NaiveDate::from_ymd_opt(2026, 5, 27).expect("date should be valid");
    let time = NaiveTime::from_hms_opt(9, 0, 0).expect("time should be valid");
    let config = alerting_config();
    let mut inspection_dispatch = InspectionDispatchState::new();
    let (delivery_tx, _delivery_rx) = mpsc::channel(1);
    delivery_tx
        .try_send(DeliveryJob::Inspection {
            config: Arc::clone(&config),
            occurred_at: now,
            local_date: date,
            lookback_hours: config.inspection.lookback_hours,
            report: report(),
        })
        .expect("prefill should fit in queue");

    enqueue_inspection_delivery(
        &delivery_tx,
        &mut inspection_dispatch,
        &config,
        report(),
        date,
        now,
    );

    assert!(
        inspection_dispatch
            .due_date_for(date, time, time, now + Duration::minutes(1))
            .is_none()
    );
    assert_eq!(
        inspection_dispatch.due_date_for(date, time, time, now + Duration::minutes(6)),
        Some(date)
    );
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
