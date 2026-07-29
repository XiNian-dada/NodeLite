use std::collections::HashMap;

use chrono::{DateTime, Datelike, TimeZone, Utc};
use nodelite_proto::{AlertMetric, AlertRuleConfig, HistoryPoint, NodeStatus};
use tracing::warn;

use crate::history::HistoryStore;
use crate::registry::NodeRegistry;
use crate::state::SharedState;

use super::super::evaluator::{
    AlertStatusView, comparator_matches, evaluate_rule, evaluate_rules, rule_matches_scope,
};
use super::super::{AlertMetricReading, EvaluatedRule};

#[derive(Debug)]
struct TrafficAlertStatus {
    node_id: String,
    node_label: String,
    tags: Vec<String>,
    usage_percent: u64,
}

impl AlertStatusView for TrafficAlertStatus {
    fn node_id(&self) -> &str {
        &self.node_id
    }

    fn node_label(&self) -> &str {
        &self.node_label
    }

    fn tags(&self) -> &[String] {
        &self.tags
    }

    fn snapshot(&self) -> Option<&nodelite_proto::NodeSnapshot> {
        None
    }

    fn last_seen(&self) -> Option<DateTime<Utc>> {
        None
    }

    fn latency_ms(&self) -> Option<u64> {
        None
    }

    fn online(&self) -> bool {
        false
    }

    fn traffic_usage_percent(&self) -> Option<u64> {
        Some(self.usage_percent)
    }
}

pub(super) async fn evaluate_alert_rules_with_history(
    shared: &SharedState,
    history: &HistoryStore,
    registry: &NodeRegistry,
    rules: &[AlertRuleConfig],
    now: DateTime<Utc>,
) -> Vec<EvaluatedRule> {
    let mut matches = if rules.iter().any(rule_uses_history_average) {
        let statuses = shared.list_statuses().await;
        let history_by_node = load_alert_history(shared, history, rules, &statuses, now).await;
        let mut matches = Vec::new();
        for rule in rules.iter().filter(|rule| rule.enabled) {
            for status in &statuses {
                if let Some(matched) =
                    evaluate_rule_with_history(rule, status, &history_by_node, now)
                {
                    matches.push(matched);
                }
            }
        }
        matches
    } else {
        shared.evaluate_alert_rules(rules, now).await
    };
    matches.extend(evaluate_traffic_usage_rules(history, registry, rules, now).await);
    matches
}

async fn evaluate_traffic_usage_rules(
    history: &HistoryStore,
    registry: &NodeRegistry,
    rules: &[AlertRuleConfig],
    now: DateTime<Utc>,
) -> Vec<EvaluatedRule> {
    if !rules
        .iter()
        .any(|rule| rule.enabled && matches!(rule.metric, AlertMetric::TrafficUsagePercent))
    {
        return Vec::new();
    }
    let month_started_at = match Utc
        .with_ymd_and_hms(now.year(), now.month(), 1, 0, 0, 0)
        .single()
    {
        Some(month_started_at) => month_started_at,
        None => now,
    };
    let usage_by_node = history
        .traffic_usages()
        .await
        .into_iter()
        .filter(|usage| usage.cycle_started_at == month_started_at)
        .map(|usage| (usage.node_id.clone(), usage))
        .collect::<HashMap<_, _>>();
    let statuses = registry
        .list_registered_nodes()
        .await
        .into_iter()
        .filter_map(|node| {
            let limit_bytes = node.traffic_limit_bytes?;
            let usage = usage_by_node.get(&node.node_id)?;
            (usage.accounting == node.traffic_accounting).then(|| TrafficAlertStatus {
                node_id: node.node_id,
                node_label: node.node_label,
                tags: node.tags,
                usage_percent: traffic_usage_percent(usage.used_bytes, limit_bytes),
            })
        })
        .collect::<Vec<_>>();
    evaluate_rules(rules, statuses.iter(), now)
}

fn traffic_usage_percent(used_bytes: u64, limit_bytes: u64) -> u64 {
    if limit_bytes == 0 {
        return u64::MAX;
    }
    let percent = (u128::from(used_bytes) * 100) / u128::from(limit_bytes);
    u64::try_from(percent).unwrap_or(u64::MAX)
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
        .filter(|rule| rule_uses_history_average(rule) && rule_matches_scope(rule, status))
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
        AlertMetric::OfflineMinutes | AlertMetric::TrafficUsagePercent => None,
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
