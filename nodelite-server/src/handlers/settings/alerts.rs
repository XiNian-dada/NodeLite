use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use chrono::Utc;
use tracing::error;

use crate::AppState;
use nodelite_proto::{
    AlertComparator, AlertMetric, AlertRuleConfig, AlertScopeMode, AlertingConfig, NodeStatus,
};

use super::config_edit::persist_alerting_change;
use super::helpers::settings_json_error;
use super::types::{
    AlertPreview, AlertRuleView, AlertSettingsResponse, AlertSettingsView, AlertSmtpSettingsView,
    AlertWebhookSettingsView, InspectionHighlight, InspectionPreview, InspectionSettingsView,
    TriggeredRulePreview, UpdateAlertSettingsRequest,
};

pub(crate) async fn alert_settings(State(state): State<AppState>) -> impl IntoResponse {
    Json(build_alert_settings_response(&state).await)
}

pub(crate) async fn update_alert_settings(
    State(state): State<AppState>,
    Json(request): Json<UpdateAlertSettingsRequest>,
) -> Response {
    let next_config = {
        let current = state.alerting.read().await;
        merge_alerting_request(&current, request)
    };

    if let Err(error) = persist_alerting_change(&state.config_path, &next_config).await {
        error!(error = ?error, path = %state.config_path.display(), "failed to persist alerting settings");
        let message = error.to_string();
        let status = if message.contains("updated server config would be invalid") {
            StatusCode::BAD_REQUEST
        } else {
            StatusCode::INTERNAL_SERVER_ERROR
        };
        return settings_json_error(status, message);
    }

    {
        let mut alerting = state.alerting.write().await;
        *alerting = next_config;
    }

    Json(build_alert_settings_response(&state).await).into_response()
}

async fn build_alert_settings_response(state: &AppState) -> AlertSettingsResponse {
    let alerting = {
        let alerting = state.alerting.read().await;
        alerting.clone()
    };
    let statuses = state.shared.list_statuses().await;
    AlertSettingsResponse {
        config: alert_settings_view(&alerting),
        preview: build_alert_preview(&alerting, &statuses),
    }
}

fn merge_alerting_request(
    current: &AlertingConfig,
    request: UpdateAlertSettingsRequest,
) -> AlertingConfig {
    AlertingConfig {
        enabled: request.enabled,
        smtp: nodelite_proto::AlertSmtpConfig {
            enabled: request.smtp.enabled,
            host: request.smtp.host,
            port: request.smtp.port,
            username: request.smtp.username,
            password: if request.smtp.clear_password {
                None
            } else {
                request
                    .smtp
                    .password
                    .filter(|value| !value.trim().is_empty())
                    .or_else(|| current.smtp.password.clone())
            },
            sender: request.smtp.sender,
            recipients: request.smtp.recipients,
            transport: request.smtp.transport,
        },
        webhook: nodelite_proto::AlertWebhookConfig {
            enabled: request.webhook.enabled,
            url: request.webhook.url,
            secret: if request.webhook.clear_secret {
                None
            } else {
                request
                    .webhook
                    .secret
                    .filter(|value| !value.trim().is_empty())
                    .or_else(|| current.webhook.secret.clone())
            },
            send_resolved: request.webhook.send_resolved,
        },
        rules: request
            .rules
            .into_iter()
            .map(|rule| nodelite_proto::AlertRuleConfig {
                id: rule.id,
                name: rule.name,
                enabled: rule.enabled,
                metric: rule.metric,
                comparator: rule.comparator,
                threshold: rule.threshold,
                window_minutes: rule.window_minutes,
                severity: rule.severity,
                scope_mode: rule.scope_mode,
                node_ids: rule.node_ids,
                tags: rule.tags,
                delivery: rule.delivery,
                cooldown_minutes: rule.cooldown_minutes,
                send_resolved: rule.send_resolved,
            })
            .collect(),
        inspection: nodelite_proto::InspectionConfig {
            enabled: request.inspection.enabled,
            local_time: request.inspection.local_time,
            lookback_hours: request.inspection.lookback_hours,
            delivery: request.inspection.delivery,
            offline_grace_minutes: request.inspection.offline_grace_minutes,
            latency_warn_ms: request.inspection.latency_warn_ms,
            cpu_warn_percent: request.inspection.cpu_warn_percent,
            memory_warn_percent: request.inspection.memory_warn_percent,
        },
    }
}

fn alert_settings_view(config: &AlertingConfig) -> AlertSettingsView {
    AlertSettingsView {
        enabled: config.enabled,
        smtp: AlertSmtpSettingsView {
            enabled: config.smtp.enabled,
            host: config.smtp.host.clone(),
            port: config.smtp.port,
            username: config.smtp.username.clone(),
            sender: config.smtp.sender.clone(),
            recipients: config.smtp.recipients.clone(),
            transport: config.smtp.transport.clone(),
            password_configured: config.smtp.password.is_some(),
        },
        webhook: AlertWebhookSettingsView {
            enabled: config.webhook.enabled,
            url: config.webhook.url.clone(),
            send_resolved: config.webhook.send_resolved,
            secret_configured: config.webhook.secret.is_some(),
        },
        rules: config
            .rules
            .iter()
            .map(|rule| AlertRuleView {
                id: rule.id.clone(),
                name: rule.name.clone(),
                enabled: rule.enabled,
                metric: rule.metric.clone(),
                comparator: rule.comparator.clone(),
                threshold: rule.threshold,
                window_minutes: rule.window_minutes,
                severity: rule.severity.clone(),
                scope_mode: rule.scope_mode.clone(),
                node_ids: rule.node_ids.clone(),
                tags: rule.tags.clone(),
                delivery: rule.delivery.clone(),
                cooldown_minutes: rule.cooldown_minutes,
                send_resolved: rule.send_resolved,
            })
            .collect(),
        inspection: InspectionSettingsView {
            enabled: config.inspection.enabled,
            local_time: config.inspection.local_time.clone(),
            lookback_hours: config.inspection.lookback_hours,
            delivery: config.inspection.delivery.clone(),
            offline_grace_minutes: config.inspection.offline_grace_minutes,
            latency_warn_ms: config.inspection.latency_warn_ms,
            cpu_warn_percent: config.inspection.cpu_warn_percent,
            memory_warn_percent: config.inspection.memory_warn_percent,
        },
    }
}

fn build_alert_preview(config: &AlertingConfig, statuses: &[NodeStatus]) -> AlertPreview {
    AlertPreview {
        generated_at: Utc::now(),
        triggered_rules: config
            .rules
            .iter()
            .filter(|rule| rule.enabled)
            .filter_map(|rule| {
                let node_ids = statuses
                    .iter()
                    .filter(|status| rule_matches_status(rule, status))
                    .map(|status| status.identity.node_id.clone())
                    .collect::<Vec<_>>();
                if node_ids.is_empty() {
                    return None;
                }
                Some(TriggeredRulePreview {
                    rule_id: rule.id.clone(),
                    rule_name: rule.name.clone(),
                    severity: rule.severity.clone(),
                    node_ids,
                })
            })
            .collect(),
        inspection: build_inspection_preview(&config.inspection, statuses),
    }
}

fn build_inspection_preview(
    inspection: &nodelite_proto::InspectionConfig,
    statuses: &[NodeStatus],
) -> InspectionPreview {
    let mut offline_nodes = 0;
    let mut latency_nodes = 0;
    let mut cpu_hot_nodes = 0;
    let mut memory_hot_nodes = 0;
    let mut highlights = Vec::new();

    for status in statuses {
        let mut reasons = Vec::new();
        if offline_minutes(status).is_some_and(|minutes| minutes >= inspection.offline_grace_minutes) {
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
        if memory_percent(status)
            .is_some_and(|memory| memory >= inspection.memory_warn_percent)
        {
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

    InspectionPreview {
        total_nodes: statuses.len(),
        offline_nodes,
        latency_nodes,
        cpu_hot_nodes,
        memory_hot_nodes,
        highlights,
    }
}

fn rule_matches_status(rule: &AlertRuleConfig, status: &NodeStatus) -> bool {
    if !rule_matches_scope(rule, status) {
        return false;
    }
    let Some(value) = metric_value(rule.metric.clone(), status) else {
        return false;
    };
    comparator_matches(rule.comparator.clone(), value, rule.threshold)
}

fn rule_matches_scope(rule: &AlertRuleConfig, status: &NodeStatus) -> bool {
    match rule.scope_mode {
        AlertScopeMode::All => true,
        AlertScopeMode::NodeIds => rule.node_ids.iter().any(|node_id| node_id == &status.identity.node_id),
        AlertScopeMode::Tags => status
            .identity
            .tags
            .iter()
            .any(|tag| rule.tags.iter().any(|rule_tag| rule_tag == tag)),
    }
}

fn metric_value(metric: AlertMetric, status: &NodeStatus) -> Option<u64> {
    match metric {
        AlertMetric::CpuUsagePercent => status
            .snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.cpu_usage_percent.map(|value| value.round() as u64)),
        AlertMetric::MemoryUsagePercent => memory_percent(status),
        AlertMetric::DiskUsagePercent => max_disk_percent(status),
        AlertMetric::LatencyMs => status.latency_ms,
        AlertMetric::OfflineMinutes => offline_minutes(status),
    }
}

fn comparator_matches(comparator: AlertComparator, left: u64, right: u64) -> bool {
    match comparator {
        AlertComparator::Gt => left >= right,
        AlertComparator::Lt => left <= right,
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

fn offline_minutes(status: &NodeStatus) -> Option<u64> {
    if status.online {
        return None;
    }
    let minutes = (Utc::now() - status.last_seen?).num_minutes();
    Some(minutes.max(0) as u64)
}

#[cfg(test)]
mod tests {
    use chrono::{Duration, Utc};

    use super::{build_alert_preview, merge_alerting_request, rule_matches_status};
    use crate::test_support::{fake_snapshot, synthetic_identity};
    use nodelite_proto::{
        AlertChannel, AlertComparator, AlertMetric, AlertRuleConfig, AlertScopeMode,
        AlertSeverity, AlertSmtpConfig, AlertSmtpTransport, AlertWebhookConfig, AlertingConfig,
        InspectionConfig, NodeStatus,
    };

    fn sample_status(node_id: &str, label: &str, online: bool, cpu: f64, latency_ms: u64) -> NodeStatus {
        let mut snapshot = fake_snapshot(300);
        snapshot.cpu_usage_percent = Some(cpu);
        snapshot.memory.used_bytes = 8;
        snapshot.memory.total_bytes = 10;
        NodeStatus {
            identity: synthetic_identity(node_id, label, "2.2.6", Some("6.8.0"), "edge"),
            snapshot: Some(snapshot),
            online,
            last_seen: Some(Utc::now() - Duration::minutes(30)),
            remote_ip: Some("203.0.113.8".to_string()),
            latency_ms: Some(latency_ms),
        }
    }

    #[test]
    fn merge_alerting_request_keeps_existing_secrets_when_not_overridden() {
        let current = AlertingConfig {
            enabled: true,
            smtp: AlertSmtpConfig {
                enabled: true,
                host: "smtp.example.com".to_string(),
                port: 587,
                username: "ops".to_string(),
                password: Some("smtp-secret".to_string()),
                sender: "ops@example.com".to_string(),
                recipients: vec!["ops@example.com".to_string()],
                transport: AlertSmtpTransport::StartTls,
            },
            webhook: AlertWebhookConfig {
                enabled: true,
                url: "https://hooks.example.com".to_string(),
                secret: Some("hook-secret".to_string()),
                send_resolved: true,
            },
            rules: Vec::new(),
            inspection: InspectionConfig::default(),
        };
        let request = super::UpdateAlertSettingsRequest {
            enabled: true,
            smtp: super::super::types::UpdateAlertSmtpSettingsRequest {
                enabled: true,
                host: "smtp.example.com".to_string(),
                port: 587,
                username: "ops".to_string(),
                password: None,
                clear_password: false,
                sender: "ops@example.com".to_string(),
                recipients: vec!["ops@example.com".to_string()],
                transport: AlertSmtpTransport::StartTls,
            },
            webhook: super::super::types::UpdateAlertWebhookSettingsRequest {
                enabled: true,
                url: "https://hooks.example.com".to_string(),
                secret: None,
                clear_secret: false,
                send_resolved: true,
            },
            rules: Vec::new(),
            inspection: super::super::types::UpdateInspectionSettingsRequest {
                enabled: false,
                local_time: "09:00".to_string(),
                lookback_hours: 24,
                delivery: vec![AlertChannel::Smtp],
                offline_grace_minutes: 10,
                latency_warn_ms: 250,
                cpu_warn_percent: 85,
                memory_warn_percent: 90,
            },
        };

        let merged = merge_alerting_request(&current, request);

        assert_eq!(merged.smtp.password.as_deref(), Some("smtp-secret"));
        assert_eq!(merged.webhook.secret.as_deref(), Some("hook-secret"));
    }

    #[test]
    fn rule_matches_status_uses_scope_and_threshold() {
        let status = sample_status("hk-01", "Hong Kong", true, 91.0, 140);
        let rule = AlertRuleConfig {
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
        };

        assert!(rule_matches_status(&rule, &status));
    }

    #[test]
    fn build_alert_preview_lists_triggered_rules_and_inspection_highlights() {
        let status = sample_status("hk-01", "Hong Kong", false, 88.0, 320);
        let config = AlertingConfig {
            enabled: true,
            smtp: AlertSmtpConfig::default(),
            webhook: AlertWebhookConfig::default(),
            rules: vec![AlertRuleConfig {
                id: "latency-hot".to_string(),
                name: "Latency".to_string(),
                enabled: true,
                metric: AlertMetric::LatencyMs,
                comparator: AlertComparator::Gt,
                threshold: 300,
                window_minutes: 5,
                severity: AlertSeverity::Warning,
                scope_mode: AlertScopeMode::All,
                node_ids: Vec::new(),
                tags: Vec::new(),
                delivery: vec![AlertChannel::Webhook],
                cooldown_minutes: 30,
                send_resolved: true,
            }],
            inspection: InspectionConfig::default(),
        };

        let preview = build_alert_preview(&config, &[status]);

        assert_eq!(preview.triggered_rules.len(), 1);
        assert_eq!(preview.inspection.offline_nodes, 1);
        assert_eq!(preview.inspection.latency_nodes, 1);
        assert_eq!(preview.inspection.highlights.len(), 1);
    }
}
