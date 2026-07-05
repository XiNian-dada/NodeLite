use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Local, NaiveDate, NaiveTime, Utc};
use nodelite_proto::{AlertChannel, AlertingConfig, HistoryPoint, InspectionConfig};
use tokio::sync::{RwLock, Semaphore, mpsc};
use tokio::task::{JoinHandle, JoinSet};
use tokio::time::{MissedTickBehavior, interval};
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use crate::history::HistoryStore;
use crate::queue::{bounded_mpsc_channel, try_enqueue};
use crate::state::SharedState;

use super::delivery::AlertDeliveryError;
use drain::drain_delivery_dispatcher;
use window::evaluate_alert_rules_with_history;

mod drain;
mod window;

use super::{
    AlertEvent, AlertEventKind, AlertStateTracker, InspectionHighlight, InspectionHighlightEvent,
    InspectionReport, InspectionSummary, InspectionTrendPoint, deliver_alert_event,
    deliver_inspection_summary, smtp_endpoint_label, webhook_endpoint_label,
};

const ALERT_EVALUATION_INTERVAL_SECS: u64 = 30;
const INSPECTION_RETRY_INTERVAL_SECS: i64 = 300;
const DELIVERY_QUEUE_CAPACITY: usize = 1024;
const MAX_CONCURRENT_DELIVERIES: usize = 8;
const INSPECTION_TREND_BUCKETS: usize = 24;

#[derive(Debug)]
enum DeliveryJob {
    Alert {
        config: Arc<AlertingConfig>,
        event: AlertEvent,
    },
    Inspection {
        config: Arc<AlertingConfig>,
        occurred_at: DateTime<Utc>,
        local_date: NaiveDate,
        lookback_hours: u64,
        report: InspectionReport,
        trends: Vec<InspectionTrendPoint>,
    },
}

#[derive(Debug)]
enum DeliveryResult {
    Alert {
        config: Arc<AlertingConfig>,
        event: AlertEvent,
        result: Result<(), AlertDeliveryError>,
    },
    Inspection {
        config: Arc<AlertingConfig>,
        local_date: NaiveDate,
        report: InspectionReport,
        result: Result<(), AlertDeliveryError>,
    },
}

pub(crate) fn spawn_alert_runtime(
    alerting: Arc<RwLock<Arc<AlertingConfig>>>,
    shared: SharedState,
    history: HistoryStore,
    shutdown: CancellationToken,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        run_alert_runtime(alerting, shared, history, shutdown).await;
    })
}

async fn run_alert_runtime(
    alerting: Arc<RwLock<Arc<AlertingConfig>>>,
    shared: SharedState,
    history: HistoryStore,
    shutdown: CancellationToken,
) {
    let mut tracker = AlertStateTracker::new();
    let mut inspection_dispatch = InspectionDispatchState::new();
    let (delivery_tx, delivery_rx) = bounded_mpsc_channel(DELIVERY_QUEUE_CAPACITY);
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
                    Arc::clone(&alerting)
                };
                if !config.enabled {
                    tracker.clear();
                    inspection_dispatch.clear();
                    continue;
                }

                let now = Utc::now();
                if config.rules.is_empty() {
                    tracker.clear();
                } else {
                    let matches =
                        evaluate_alert_rules_with_history(&shared, &history, &config.rules, now)
                            .await;
                    for event in tracker.update(&config.rules, &matches, now) {
                        log_alert_event(&event);
                        enqueue_alert_delivery(&delivery_tx, &mut tracker, &config, &event, now);
                    }
                }

                if should_check_inspection(&config)
                    && let Some(local_date) =
                        inspection_dispatch.due_date(&config.inspection.local_time, Local::now(), now)
                {
                    let mut report = shared
                        .build_alert_inspection_report(&config.inspection, now)
                        .await;
                    let history_analysis = build_inspection_history_analysis(
                        &shared,
                        &history,
                        &config.inspection,
                        now,
                    )
                    .await;
                    report.highlights =
                        merge_inspection_highlights(report.highlights, history_analysis.highlights);
                    enqueue_inspection_delivery(
                        &delivery_tx,
                        &mut inspection_dispatch,
                        &config,
                        report,
                        history_analysis.trends,
                        local_date,
                        now,
                    );
                }
            }
        }
    }
    drop(delivery_tx);
    drain_delivery_dispatcher(delivery_dispatcher).await;
}

fn spawn_delivery_dispatcher(
    mut delivery_rx: mpsc::Receiver<DeliveryJob>,
    result_tx: mpsc::UnboundedSender<DeliveryResult>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let limiter = Arc::new(Semaphore::new(MAX_CONCURRENT_DELIVERIES));
        let mut deliveries = JoinSet::new();
        loop {
            tokio::select! {
                Some(job) = delivery_rx.recv() => {
                    let limiter = Arc::clone(&limiter);
                    let result_tx = result_tx.clone();
                    deliveries.spawn(async move {
                        let Ok(_permit) = limiter.acquire_owned().await else {
                            return;
                        };
                        let result = deliver_job(job).await;
                        let _ = result_tx.send(result);
                    });
                }
                Some(result) = deliveries.join_next(), if !deliveries.is_empty() => {
                    log_delivery_task_join(result);
                }
                else => break,
            }
        }

        while let Some(result) = deliveries.join_next().await {
            log_delivery_task_join(result);
        }
    })
}

fn log_delivery_task_join(result: Result<(), tokio::task::JoinError>) {
    if let Err(error) = result {
        warn!(error = ?error, "alert delivery task join failed");
    }
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
    config: &Arc<AlertingConfig>,
    event: &AlertEvent,
    now: DateTime<Utc>,
) {
    if try_enqueue(
        delivery_tx,
        DeliveryJob::Alert {
            config: Arc::clone(config),
            event: event.clone(),
        },
    )
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
    config: &Arc<AlertingConfig>,
    report: InspectionReport,
    trends: Vec<InspectionTrendPoint>,
    local_date: NaiveDate,
    now: DateTime<Utc>,
) {
    if try_enqueue(
        delivery_tx,
        DeliveryJob::Inspection {
            config: Arc::clone(config),
            occurred_at: now,
            local_date,
            lookback_hours: config.inspection.lookback_hours,
            report,
            trends,
        },
    )
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

#[derive(Debug, Default)]
struct InspectionHistoryAnalysis {
    trends: Vec<InspectionTrendPoint>,
    highlights: Vec<InspectionHighlight>,
}

async fn build_inspection_history_analysis(
    shared: &SharedState,
    history: &HistoryStore,
    inspection: &InspectionConfig,
    now: DateTime<Utc>,
) -> InspectionHistoryAnalysis {
    let bucket_count = inspection_trend_bucket_count(inspection.lookback_hours);
    let since = now - chrono::Duration::hours(inspection.lookback_hours.max(1) as i64);
    let span_seconds = (now.timestamp() - since.timestamp()).max(1);
    let bucket_seconds = ((span_seconds as usize).div_ceil(bucket_count)).max(1) as i64;
    let mut buckets = (0..bucket_count)
        .map(|index| TrendBucket::new(trend_label(since, bucket_seconds, index)))
        .collect::<Vec<_>>();
    let mut highlights = Vec::new();

    for status in shared.list_statuses().await {
        let node_id = status.identity.node_id.clone();
        let node_label = status.identity.node_label.clone();
        match history
            .query_history_range(&node_id, since, now, bucket_count)
            .await
        {
            Ok(points) => {
                let mut history_highlight =
                    HistoryHighlightBuilder::new(node_id.clone(), node_label);
                for point in points {
                    let index = ((point.recorded_at.timestamp() - since.timestamp()).max(0)
                        / bucket_seconds) as usize;
                    let index = index.min(bucket_count.saturating_sub(1));
                    buckets[index].record(&point);
                    history_highlight.record(&point, inspection);
                }
                if let Some(highlight) = history_highlight.finish(inspection) {
                    highlights.push(highlight);
                }
            }
            Err(error) => warn!(
                node_id = %node_id,
                error = ?error,
                "failed to load history for daily inspection trend",
            ),
        }
    }

    InspectionHistoryAnalysis {
        trends: buckets.into_iter().map(TrendBucket::finish).collect(),
        highlights,
    }
}

fn merge_inspection_highlights(
    mut current: Vec<InspectionHighlight>,
    history: Vec<InspectionHighlight>,
) -> Vec<InspectionHighlight> {
    for mut highlight in history {
        if let Some(existing) = current
            .iter_mut()
            .find(|existing| existing.node_id == highlight.node_id)
        {
            existing.events.append(&mut highlight.events);
        } else {
            current.push(highlight);
        }
    }
    for highlight in &mut current {
        highlight.events.sort_by(|left, right| {
            right
                .occurred_at
                .cmp(&left.occurred_at)
                .then_with(|| left.summary.cmp(&right.summary))
        });
        highlight.events.dedup_by(|left, right| {
            left.occurred_at == right.occurred_at && left.summary == right.summary
        });
        highlight.events.truncate(6);
    }
    current.sort_by(|left, right| {
        latest_highlight_time(right)
            .cmp(&latest_highlight_time(left))
            .then_with(|| left.node_label.cmp(&right.node_label))
    });
    current
}

fn latest_highlight_time(highlight: &InspectionHighlight) -> DateTime<Utc> {
    highlight
        .events
        .iter()
        .map(|event| event.occurred_at)
        .max()
        .unwrap_or_else(Utc::now)
}

fn inspection_trend_bucket_count(lookback_hours: u64) -> usize {
    (lookback_hours.max(1) as usize).min(INSPECTION_TREND_BUCKETS)
}

fn trend_label(since: DateTime<Utc>, bucket_seconds: i64, index: usize) -> String {
    (since + chrono::Duration::seconds(bucket_seconds.saturating_mul(index as i64)))
        .format("%H:%M")
        .to_string()
}

#[derive(Debug)]
struct TrendBucket {
    label: String,
    cpu_sum: f64,
    cpu_count: u64,
    memory_sum: f64,
    memory_count: u64,
    latency_sum: u64,
    latency_count: u64,
}

impl TrendBucket {
    fn new(label: String) -> Self {
        Self {
            label,
            cpu_sum: 0.0,
            cpu_count: 0,
            memory_sum: 0.0,
            memory_count: 0,
            latency_sum: 0,
            latency_count: 0,
        }
    }

    fn record(&mut self, point: &nodelite_proto::HistoryPoint) {
        if let Some(cpu) = point.cpu_usage_percent
            && cpu.is_finite()
        {
            self.cpu_sum += cpu;
            self.cpu_count += 1;
        }
        if point.memory_used_percent.is_finite() {
            self.memory_sum += point.memory_used_percent;
            self.memory_count += 1;
        }
        if let Some(latency) = point.latency_ms {
            self.latency_sum = self.latency_sum.saturating_add(latency);
            self.latency_count += 1;
        }
    }

    fn finish(self) -> InspectionTrendPoint {
        InspectionTrendPoint {
            label: self.label,
            cpu_usage_percent: average_percent(self.cpu_sum, self.cpu_count),
            memory_used_percent: average_percent(self.memory_sum, self.memory_count),
            latency_ms: average_u64(self.latency_sum, self.latency_count),
        }
    }
}

fn average_percent(sum: f64, count: u64) -> Option<u64> {
    (count > 0).then(|| ((sum / count as f64).round() as u64).min(100))
}

fn average_u64(sum: u64, count: u64) -> Option<u64> {
    (count > 0).then(|| (sum as f64 / count as f64).round() as u64)
}

#[derive(Debug)]
struct HistoryHighlightBuilder {
    node_id: String,
    node_label: String,
    cpu_peak: Option<MetricPeak>,
    memory_peak: Option<MetricPeak>,
    latency_peak: Option<MetricPeak>,
}

impl HistoryHighlightBuilder {
    fn new(node_id: String, node_label: String) -> Self {
        Self {
            node_id,
            node_label,
            cpu_peak: None,
            memory_peak: None,
            latency_peak: None,
        }
    }

    fn record(&mut self, point: &HistoryPoint, inspection: &InspectionConfig) {
        if let Some(cpu) = point.cpu_usage_percent
            && cpu.is_finite()
        {
            self.record_cpu(point.recorded_at, cpu.round() as u64, inspection);
        }
        if point.memory_used_percent.is_finite() {
            self.record_memory(
                point.recorded_at,
                point.memory_used_percent.round() as u64,
                inspection,
            );
        }
        if let Some(latency) = point.latency_ms {
            self.record_latency(point.recorded_at, latency, inspection);
        }
    }

    fn record_cpu(
        &mut self,
        occurred_at: DateTime<Utc>,
        value: u64,
        inspection: &InspectionConfig,
    ) {
        if value >= inspection.cpu_warn_percent {
            update_peak(&mut self.cpu_peak, occurred_at, value);
        }
    }

    fn record_memory(
        &mut self,
        occurred_at: DateTime<Utc>,
        value: u64,
        inspection: &InspectionConfig,
    ) {
        if value >= inspection.memory_warn_percent {
            update_peak(&mut self.memory_peak, occurred_at, value.min(100));
        }
    }

    fn record_latency(
        &mut self,
        occurred_at: DateTime<Utc>,
        value: u64,
        inspection: &InspectionConfig,
    ) {
        if value >= inspection.latency_warn_ms {
            update_peak(&mut self.latency_peak, occurred_at, value);
        }
    }

    fn finish(self, inspection: &InspectionConfig) -> Option<InspectionHighlight> {
        let mut events = Vec::new();
        if let Some(peak) = self.cpu_peak {
            events.push(InspectionHighlightEvent {
                occurred_at: peak.occurred_at,
                summary: format!(
                    "CPU peaked at {}% (warn >= {}%)",
                    peak.value, inspection.cpu_warn_percent
                ),
            });
        }
        if let Some(peak) = self.memory_peak {
            events.push(InspectionHighlightEvent {
                occurred_at: peak.occurred_at,
                summary: format!(
                    "Memory peaked at {}% (warn >= {}%)",
                    peak.value, inspection.memory_warn_percent
                ),
            });
        }
        if let Some(peak) = self.latency_peak {
            events.push(InspectionHighlightEvent {
                occurred_at: peak.occurred_at,
                summary: format!(
                    "RTT peaked at {} ms (warn >= {} ms)",
                    peak.value, inspection.latency_warn_ms
                ),
            });
        }
        if events.is_empty() {
            return None;
        }
        Some(InspectionHighlight {
            node_id: self.node_id,
            node_label: self.node_label,
            events,
        })
    }
}

#[derive(Debug, Clone, Copy)]
struct MetricPeak {
    occurred_at: DateTime<Utc>,
    value: u64,
}

fn update_peak(peak: &mut Option<MetricPeak>, occurred_at: DateTime<Utc>, value: u64) {
    if peak.is_none_or(|peak| {
        value > peak.value || value == peak.value && occurred_at > peak.occurred_at
    }) {
        *peak = Some(MetricPeak { occurred_at, value });
    }
}

fn handle_alert_delivery_result(
    tracker: &mut AlertStateTracker,
    delivery_tx: &mpsc::Sender<DeliveryJob>,
    config: &Arc<AlertingConfig>,
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
mod tests;
