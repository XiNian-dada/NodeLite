use chrono::{DateTime, Utc};
use nodelite_proto::{
    AlertComparator, AlertMetric, AlertRuleConfig, AlertScopeMode, InspectionConfig, NodeStatus,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AlertMetricReading {
    pub(crate) metric: AlertMetric,
    pub(crate) value: u64,
    pub(crate) threshold: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EvaluatedRule {
    pub(crate) rule_id: String,
    pub(crate) node_id: String,
    pub(crate) node_label: String,
    pub(crate) reading: AlertMetricReading,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct InspectionReport {
    pub(crate) total_nodes: usize,
    pub(crate) offline_nodes: usize,
    pub(crate) latency_nodes: usize,
    pub(crate) cpu_hot_nodes: usize,
    pub(crate) memory_hot_nodes: usize,
    pub(crate) highlights: Vec<InspectionHighlight>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct InspectionHighlight {
    pub(crate) node_id: String,
    pub(crate) node_label: String,
    pub(crate) reasons: Vec<String>,
}

pub(crate) fn evaluate_rules(
    rules: &[AlertRuleConfig],
    statuses: &[NodeStatus],
    now: DateTime<Utc>,
) -> Vec<EvaluatedRule> {
    let mut matches = Vec::new();
    for rule in rules.iter().filter(|rule| rule.enabled) {
        matches.extend(
            statuses
                .iter()
                .filter_map(|status| evaluate_rule(rule, status, now)),
        );
    }
    matches
}

pub(crate) fn evaluate_rule(
    rule: &AlertRuleConfig,
    status: &NodeStatus,
    now: DateTime<Utc>,
) -> Option<EvaluatedRule> {
    let value = rule_metric_value(rule, status, now)?;
    if !comparator_matches(rule.comparator.clone(), value, rule.threshold) {
        return None;
    }
    Some(EvaluatedRule {
        rule_id: rule.id.clone(),
        node_id: status.identity.node_id.clone(),
        node_label: status.identity.node_label.clone(),
        reading: AlertMetricReading {
            metric: rule.metric.clone(),
            value,
            threshold: rule.threshold,
        },
    })
}

pub(crate) fn build_inspection_report(
    inspection: &InspectionConfig,
    statuses: &[NodeStatus],
    now: DateTime<Utc>,
) -> InspectionReport {
    let mut offline_nodes = 0;
    let mut latency_nodes = 0;
    let mut cpu_hot_nodes = 0;
    let mut memory_hot_nodes = 0;
    let mut highlights = Vec::new();

    for status in statuses {
        let mut reasons = Vec::new();
        if offline_minutes(status, now)
            .is_some_and(|minutes| minutes >= inspection.offline_grace_minutes)
        {
            offline_nodes += 1;
            reasons.push("offline".to_string());
        }
        if status
            .latency_ms
            .is_some_and(|latency| latency >= inspection.latency_warn_ms)
        {
            latency_nodes += 1;
            reasons.push("latency".to_string());
        }
        if status
            .snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.cpu_usage_percent)
            .is_some_and(|cpu| cpu >= inspection.cpu_warn_percent as f64)
        {
            cpu_hot_nodes += 1;
            reasons.push("cpu".to_string());
        }
        if memory_percent(status).is_some_and(|memory| memory >= inspection.memory_warn_percent) {
            memory_hot_nodes += 1;
            reasons.push("memory".to_string());
        }

        if reasons.is_empty() {
            continue;
        }
        highlights.push(InspectionHighlight {
            node_id: status.identity.node_id.clone(),
            node_label: status.identity.node_label.clone(),
            reasons,
        });
    }

    InspectionReport {
        total_nodes: statuses.len(),
        offline_nodes,
        latency_nodes,
        cpu_hot_nodes,
        memory_hot_nodes,
        highlights,
    }
}

fn rule_metric_value(
    rule: &AlertRuleConfig,
    status: &NodeStatus,
    now: DateTime<Utc>,
) -> Option<u64> {
    if !rule_matches_scope(rule, status) {
        return None;
    }
    metric_value(rule.metric.clone(), status, now)
}

fn rule_matches_scope(rule: &AlertRuleConfig, status: &NodeStatus) -> bool {
    match rule.scope_mode {
        AlertScopeMode::All => true,
        AlertScopeMode::NodeIds => rule
            .node_ids
            .iter()
            .any(|node_id| node_id == &status.identity.node_id),
        AlertScopeMode::Tags => status
            .identity
            .tags
            .iter()
            .any(|tag| rule.tags.iter().any(|rule_tag| rule_tag == tag)),
    }
}

fn metric_value(metric: AlertMetric, status: &NodeStatus, now: DateTime<Utc>) -> Option<u64> {
    match metric {
        AlertMetric::CpuUsagePercent => status
            .snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.cpu_usage_percent.map(|value| value.round() as u64)),
        AlertMetric::MemoryUsagePercent => memory_percent(status),
        AlertMetric::DiskUsagePercent => max_disk_percent(status),
        AlertMetric::LatencyMs => status.latency_ms,
        AlertMetric::OfflineMinutes => offline_minutes(status, now),
    }
}

fn comparator_matches(comparator: AlertComparator, left: u64, right: u64) -> bool {
    match comparator {
        AlertComparator::Gt => left > right,
        AlertComparator::Lt => left < right,
    }
}

fn memory_percent(status: &NodeStatus) -> Option<u64> {
    let memory = &status.snapshot.as_ref()?.memory;
    if memory.total_bytes == 0 {
        return None;
    }
    Some(((memory.used_bytes.saturating_mul(100)) / memory.total_bytes).min(100))
}

fn max_disk_percent(status: &NodeStatus) -> Option<u64> {
    status
        .snapshot
        .as_ref()?
        .disks
        .iter()
        .filter(|disk| disk.total_bytes > 0)
        .map(|disk| ((disk.used_bytes.saturating_mul(100)) / disk.total_bytes).min(100))
        .max()
}

fn offline_minutes(status: &NodeStatus, now: DateTime<Utc>) -> Option<u64> {
    if status.online {
        return None;
    }
    let minutes = (now - status.last_seen?).num_minutes();
    Some(minutes.max(0) as u64)
}

#[cfg(test)]
mod tests {
    use chrono::{Duration, Utc};
    use nodelite_proto::{
        AlertChannel, AlertComparator, AlertMetric, AlertRuleConfig, AlertScopeMode, AlertSeverity,
        InspectionConfig, NodeStatus,
    };

    use super::{build_inspection_report, evaluate_rule};
    use crate::test_support::{fake_snapshot, synthetic_identity};

    fn sample_status(
        now: chrono::DateTime<Utc>,
        node_id: &str,
        label: &str,
        online: bool,
        cpu: f64,
        latency_ms: u64,
    ) -> NodeStatus {
        let mut snapshot = fake_snapshot(300);
        snapshot.cpu_usage_percent = Some(cpu);
        snapshot.memory.used_bytes = 8;
        snapshot.memory.total_bytes = 10;
        NodeStatus {
            identity: synthetic_identity(node_id, label, "2.2.6", Some("6.8.0"), "edge"),
            snapshot: Some(snapshot),
            online,
            last_seen: Some(now - Duration::minutes(30)),
            remote_ip: Some("203.0.113.8".to_string()),
            geoip_country: None,
            geoip_city: None,
            geoip_latitude: None,
            geoip_longitude: None,
            latency_ms: Some(latency_ms),
        }
    }

    fn cpu_rule() -> AlertRuleConfig {
        AlertRuleConfig {
            id: "cpu-hot".to_string(),
            name: "CPU".to_string(),
            enabled: true,
            metric: AlertMetric::CpuUsagePercent,
            comparator: AlertComparator::Gt,
            threshold: 90,
            window_minutes: 5,
            severity: AlertSeverity::Critical,
            scope_mode: AlertScopeMode::Tags,
            node_ids: Vec::new(),
            tags: vec!["edge".to_string()],
            delivery: vec![AlertChannel::Smtp],
            cooldown_minutes: 30,
            send_resolved: true,
        }
    }

    #[test]
    fn evaluate_rule_uses_scope_and_threshold() {
        let now = Utc::now();
        let status = sample_status(now, "hk-01", "Hong Kong", true, 91.0, 140);
        let matched = evaluate_rule(&cpu_rule(), &status, now).expect("rule should match");

        assert_eq!(matched.node_id, "hk-01");
        assert_eq!(matched.node_label, "Hong Kong");
        assert_eq!(matched.reading.value, 91);
        assert_eq!(matched.reading.threshold, 90);
    }

    #[test]
    fn evaluate_rule_requires_strict_threshold_comparisons() {
        let now = Utc::now();
        let status = sample_status(now, "hk-01", "Hong Kong", true, 90.0, 140);

        let mut gt_rule = cpu_rule();
        gt_rule.comparator = AlertComparator::Gt;
        assert!(
            evaluate_rule(&gt_rule, &status, now).is_none(),
            "equal values must not trigger gt rules"
        );

        let mut lt_rule = cpu_rule();
        lt_rule.comparator = AlertComparator::Lt;
        lt_rule.threshold = 90;
        assert!(
            evaluate_rule(&lt_rule, &status, now).is_none(),
            "equal values must not trigger lt rules"
        );
    }

    #[test]
    fn build_inspection_report_counts_highlights() {
        let now = Utc::now();
        let status = sample_status(now, "hk-01", "Hong Kong", false, 88.0, 320);
        let report = build_inspection_report(&InspectionConfig::default(), &[status], now);

        assert_eq!(report.total_nodes, 1);
        assert_eq!(report.offline_nodes, 1);
        assert_eq!(report.latency_nodes, 1);
        assert_eq!(report.highlights.len(), 1);
    }
}
