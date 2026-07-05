use std::collections::HashMap;

use chrono::{DateTime, Utc};
use nodelite_proto::{AlertMetric, AlertRuleConfig, HistoryPoint, NodeStatus};
use tracing::warn;

use crate::history::HistoryStore;
use crate::state::SharedState;

use super::super::evaluator::{comparator_matches, evaluate_rule, rule_matches_scope};
use super::super::{AlertMetricReading, EvaluatedRule};

pub(super) async fn evaluate_alert_rules_with_history(
    shared: &SharedState,
    history: &HistoryStore,
    rules: &[AlertRuleConfig],
    now: DateTime<Utc>,
) -> Vec<EvaluatedRule> {
    if !rules.iter().any(rule_uses_history_average) {
        return shared.evaluate_alert_rules(rules, now).await;
    }

    let statuses = shared.list_statuses().await;
    let history_by_node = load_alert_history(shared, history, rules, &statuses, now).await;
    let mut matches = Vec::new();
    for rule in rules.iter().filter(|rule| rule.enabled) {
        for status in &statuses {
            if let Some(matched) = evaluate_rule_with_history(rule, status, &history_by_node, now) {
                matches.push(matched);
            }
        }
    }
    matches
}

async fn load_alert_history(
    _shared: &SharedState,
    history: &HistoryStore,
    rules: &[AlertRuleConfig],
    statuses: &[NodeStatus],
    now: DateTime<Utc>,
) -> HashMap<String, Vec<HistoryPoint>> {
    let mut history_by_node = HashMap::new();
    for status in statuses {
        let Some(window_minutes) = max_history_window_for_status(rules, status) else {
            continue;
        };
        let node_id = status.identity.node_id.clone();
        let since = now - chrono::Duration::minutes(window_minutes as i64);
        match history
            .query_history_range(&node_id, since, now, window_minutes as usize * 2)
            .await
        {
            Ok(points) => {
                history_by_node.insert(node_id, points);
            }
            Err(error) => warn!(
                node_id = %node_id,
                error = ?error,
                "failed to load history for alert rule window",
            ),
        }
    }
    history_by_node
}

fn max_history_window_for_status(rules: &[AlertRuleConfig], status: &NodeStatus) -> Option<u64> {
    rules
        .iter()
        .filter(|rule| rule_uses_history_average(rule) && rule_matches_scope(*rule, status))
        .map(|rule| rule.window_minutes)
        .max()
}

pub(super) fn evaluate_rule_with_history(
    rule: &AlertRuleConfig,
    status: &NodeStatus,
    history_by_node: &HashMap<String, Vec<HistoryPoint>>,
    now: DateTime<Utc>,
) -> Option<EvaluatedRule> {
    if rule_uses_history_average(rule) && rule_matches_scope(rule, status) {
        let since = now - chrono::Duration::minutes(rule.window_minutes as i64);
        if let Some(value) = history_by_node
            .get(&status.identity.node_id)
            .and_then(|points| average_history_metric(&rule.metric, points, since))
        {
            if comparator_matches(rule.comparator.clone(), value, rule.threshold) {
                return Some(EvaluatedRule {
                    rule_id: rule.id.clone(),
                    node_id: status.identity.node_id.clone(),
                    node_label: status.identity.node_label.clone(),
                    reading: AlertMetricReading {
                        metric: rule.metric.clone(),
                        value,
                        threshold: rule.threshold,
                    },
                });
            }
            return None;
        }
    }
    evaluate_rule(rule, status, now)
}

fn rule_uses_history_average(rule: &AlertRuleConfig) -> bool {
    rule.enabled
        && rule.window_minutes > 1
        && matches!(
            rule.metric,
            AlertMetric::CpuUsagePercent
                | AlertMetric::MemoryUsagePercent
                | AlertMetric::DiskUsagePercent
                | AlertMetric::LatencyMs
        )
}

pub(super) fn average_history_metric(
    metric: &AlertMetric,
    points: &[HistoryPoint],
    since: DateTime<Utc>,
) -> Option<u64> {
    match metric {
        AlertMetric::CpuUsagePercent => average_history_percent(
            points
                .iter()
                .filter(|point| point.recorded_at >= since)
                .filter_map(|point| point.cpu_usage_percent),
        ),
        AlertMetric::MemoryUsagePercent => average_history_percent(
            points
                .iter()
                .filter(|point| point.recorded_at >= since)
                .map(|point| point.memory_used_percent),
        ),
        AlertMetric::DiskUsagePercent => average_history_percent(
            points
                .iter()
                .filter(|point| point.recorded_at >= since)
                .filter_map(|point| point.disk_used_percent),
        ),
        AlertMetric::LatencyMs => average_history_u64(
            points
                .iter()
                .filter(|point| point.recorded_at >= since)
                .filter_map(|point| point.latency_ms),
        ),
        AlertMetric::OfflineMinutes => None,
    }
}

fn average_history_percent(values: impl Iterator<Item = f64>) -> Option<u64> {
    let (sum, count) = values
        .filter(|value| value.is_finite())
        .fold((0.0, 0_u64), |(sum, count), value| (sum + value, count + 1));
    (count > 0).then(|| ((sum / count as f64).round() as u64).min(100))
}

fn average_history_u64(values: impl Iterator<Item = u64>) -> Option<u64> {
    let (sum, count) = values.fold((0_u128, 0_u64), |(sum, count), value| {
        (sum + value as u128, count + 1)
    });
    (count > 0).then(|| ((sum as f64) / count as f64).round() as u64)
}
