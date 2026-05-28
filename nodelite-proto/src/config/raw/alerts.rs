use serde::Deserialize;

use crate::validation::{normalize_string_list, validate_identifier, validate_non_empty};

use super::super::defaults::{
    default_alert_inspection_cpu_warn_percent, default_alert_inspection_latency_warn_ms,
    default_alert_inspection_local_time, default_alert_inspection_lookback_hours,
    default_alert_inspection_memory_warn_percent, default_alert_inspection_offline_grace_minutes,
    default_alert_rule_cooldown_minutes, default_alert_rule_window_minutes,
};
use super::super::helpers::{normalize_tags, validate_url};
use super::super::{
    AlertChannel, AlertComparator, AlertMetric, AlertRuleConfig, AlertScopeMode, AlertSeverity,
    AlertSmtpConfig, AlertSmtpTransport, AlertWebhookConfig, AlertingConfig, ConfigError,
    InspectionConfig,
};

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub(super) struct RawAlertsSection {
    #[serde(default)]
    enabled: bool,
    #[serde(default)]
    smtp: RawAlertSmtpSection,
    #[serde(default)]
    webhook: RawAlertWebhookSection,
    #[serde(default)]
    rules: Vec<RawAlertRuleSection>,
    #[serde(default)]
    inspection: RawInspectionSection,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawAlertSmtpSection {
    #[serde(default)]
    enabled: bool,
    #[serde(default)]
    host: String,
    #[serde(default = "default_alert_smtp_port")]
    port: u16,
    #[serde(default)]
    username: String,
    #[serde(default)]
    password: Option<String>,
    #[serde(default)]
    sender: String,
    #[serde(default)]
    recipients: Vec<String>,
    #[serde(default = "default_alert_smtp_transport")]
    transport: AlertSmtpTransport,
    #[serde(default = "default_alert_send_resolved")]
    send_resolved: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawAlertWebhookSection {
    #[serde(default)]
    enabled: bool,
    #[serde(default)]
    url: String,
    #[serde(default)]
    secret: Option<String>,
    #[serde(default = "default_alert_send_resolved")]
    send_resolved: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawAlertRuleSection {
    id: String,
    name: String,
    #[serde(default = "default_true")]
    enabled: bool,
    metric: AlertMetric,
    comparator: AlertComparator,
    threshold: u64,
    #[serde(default = "default_alert_rule_window_minutes")]
    window_minutes: u64,
    severity: AlertSeverity,
    #[serde(default = "default_alert_scope_mode")]
    scope_mode: AlertScopeMode,
    #[serde(default)]
    node_ids: Vec<String>,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    delivery: Vec<AlertChannel>,
    #[serde(default = "default_alert_rule_cooldown_minutes")]
    cooldown_minutes: u64,
    #[serde(default = "default_alert_send_resolved")]
    send_resolved: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawInspectionSection {
    #[serde(default)]
    enabled: bool,
    #[serde(default = "default_alert_inspection_local_time")]
    local_time: String,
    #[serde(default = "default_alert_inspection_lookback_hours")]
    lookback_hours: u64,
    #[serde(default = "default_inspection_delivery")]
    delivery: Vec<AlertChannel>,
    #[serde(default = "default_alert_inspection_offline_grace_minutes")]
    offline_grace_minutes: u64,
    #[serde(default = "default_alert_inspection_latency_warn_ms")]
    latency_warn_ms: u64,
    #[serde(default = "default_alert_inspection_cpu_warn_percent")]
    cpu_warn_percent: u64,
    #[serde(default = "default_alert_inspection_memory_warn_percent")]
    memory_warn_percent: u64,
}

impl Default for RawAlertSmtpSection {
    fn default() -> Self {
        Self {
            enabled: false,
            host: String::new(),
            port: default_alert_smtp_port(),
            username: String::new(),
            password: None,
            sender: String::new(),
            recipients: Vec::new(),
            transport: default_alert_smtp_transport(),
            send_resolved: default_alert_send_resolved(),
        }
    }
}

impl Default for RawAlertWebhookSection {
    fn default() -> Self {
        Self {
            enabled: false,
            url: String::new(),
            secret: None,
            send_resolved: default_alert_send_resolved(),
        }
    }
}

impl Default for RawInspectionSection {
    fn default() -> Self {
        Self {
            enabled: false,
            local_time: default_alert_inspection_local_time(),
            lookback_hours: default_alert_inspection_lookback_hours(),
            delivery: default_inspection_delivery(),
            offline_grace_minutes: default_alert_inspection_offline_grace_minutes(),
            latency_warn_ms: default_alert_inspection_latency_warn_ms(),
            cpu_warn_percent: default_alert_inspection_cpu_warn_percent(),
            memory_warn_percent: default_alert_inspection_memory_warn_percent(),
        }
    }
}

impl RawAlertsSection {
    pub(super) fn validate(&self) -> Result<AlertingConfig, ConfigError> {
        Ok(AlertingConfig {
            enabled: self.enabled,
            smtp: self.validate_smtp()?,
            webhook: self.validate_webhook()?,
            rules: self.validate_rules()?,
            inspection: self.validate_inspection()?,
        })
    }

    fn validate_smtp(&self) -> Result<AlertSmtpConfig, ConfigError> {
        let host = self.smtp.host.trim().to_string();
        let username = self.smtp.username.trim().to_string();
        let sender = self.smtp.sender.trim().to_string();
        let recipients = normalize_string_list(self.smtp.recipients.clone());
        let password = normalize_optional_trimmed(self.smtp.password.clone());
        if self.smtp.enabled {
            validate_non_empty("alerts.smtp.host", &host)?;
            validate_email_address("alerts.smtp.sender", &sender)?;
            if recipients.is_empty() {
                return Err(ConfigError::new(
                    "alerts.smtp.recipients must contain at least one recipient",
                ));
            }
            for (index, recipient) in recipients.iter().enumerate() {
                validate_email_address(&format!("alerts.smtp.recipients[{index}]"), recipient)?;
            }
        }
        if self.smtp.port == 0 {
            return Err(ConfigError::new("alerts.smtp.port must be greater than 0"));
        }

        Ok(AlertSmtpConfig {
            enabled: self.smtp.enabled,
            host,
            port: self.smtp.port,
            username,
            password,
            sender,
            recipients,
            transport: self.smtp.transport.clone(),
            send_resolved: self.smtp.send_resolved,
        })
    }

    fn validate_webhook(&self) -> Result<AlertWebhookConfig, ConfigError> {
        let url = self.webhook.url.trim().to_string();
        if self.webhook.enabled {
            validate_non_empty("alerts.webhook.url", &url)?;
            validate_url("alerts.webhook.url", &url, &["http", "https"])?;
        }

        Ok(AlertWebhookConfig {
            enabled: self.webhook.enabled,
            url,
            secret: normalize_optional_trimmed(self.webhook.secret.clone()),
            send_resolved: self.webhook.send_resolved,
        })
    }

    fn validate_rules(&self) -> Result<Vec<AlertRuleConfig>, ConfigError> {
        if self.rules.len() > 64 {
            return Err(ConfigError::new(
                "alerts.rules must contain at most 64 rules",
            ));
        }

        let mut rules = Vec::with_capacity(self.rules.len());
        for (index, rule) in self.rules.iter().enumerate() {
            let id = rule.id.trim().to_string();
            let name = rule.name.trim().to_string();
            validate_identifier(&format!("alerts.rules[{index}].id"), &id)?;
            validate_non_empty(&format!("alerts.rules[{index}].name"), &name)?;
            if rule.window_minutes == 0 {
                return Err(ConfigError::new(format!(
                    "alerts.rules[{index}].window_minutes must be greater than 0"
                )));
            }
            if rule.cooldown_minutes == 0 {
                return Err(ConfigError::new(format!(
                    "alerts.rules[{index}].cooldown_minutes must be greater than 0"
                )));
            }

            let node_ids =
                normalize_node_ids(&format!("alerts.rules[{index}].node_ids"), &rule.node_ids)?;
            let tags = normalize_tags(&format!("alerts.rules[{index}].tags"), rule.tags.clone())?;
            match rule.scope_mode {
                AlertScopeMode::All => {}
                AlertScopeMode::NodeIds if node_ids.is_empty() => {
                    return Err(ConfigError::new(format!(
                        "alerts.rules[{index}].node_ids must not be empty when scope_mode = node_ids"
                    )));
                }
                AlertScopeMode::Tags if tags.is_empty() => {
                    return Err(ConfigError::new(format!(
                        "alerts.rules[{index}].tags must not be empty when scope_mode = tags"
                    )));
                }
                _ => {}
            }

            rules.push(AlertRuleConfig {
                id,
                name,
                enabled: rule.enabled,
                metric: rule.metric.clone(),
                comparator: rule.comparator.clone(),
                threshold: rule.threshold,
                window_minutes: rule.window_minutes,
                severity: rule.severity.clone(),
                scope_mode: rule.scope_mode.clone(),
                node_ids,
                tags,
                delivery: dedup_alert_channels(rule.delivery.clone()),
                cooldown_minutes: rule.cooldown_minutes,
                send_resolved: rule.send_resolved,
            });
        }

        Ok(rules)
    }

    fn validate_inspection(&self) -> Result<InspectionConfig, ConfigError> {
        validate_local_time("alerts.inspection.local_time", &self.inspection.local_time)?;
        if self.inspection.lookback_hours == 0 {
            return Err(ConfigError::new(
                "alerts.inspection.lookback_hours must be greater than 0",
            ));
        }
        if self.inspection.cpu_warn_percent == 0 || self.inspection.cpu_warn_percent > 100 {
            return Err(ConfigError::new(
                "alerts.inspection.cpu_warn_percent must be between 1 and 100",
            ));
        }
        if self.inspection.memory_warn_percent == 0 || self.inspection.memory_warn_percent > 100 {
            return Err(ConfigError::new(
                "alerts.inspection.memory_warn_percent must be between 1 and 100",
            ));
        }

        Ok(InspectionConfig {
            enabled: self.inspection.enabled,
            local_time: self.inspection.local_time.trim().to_string(),
            lookback_hours: self.inspection.lookback_hours,
            delivery: dedup_alert_channels(self.inspection.delivery.clone()),
            offline_grace_minutes: self.inspection.offline_grace_minutes,
            latency_warn_ms: self.inspection.latency_warn_ms,
            cpu_warn_percent: self.inspection.cpu_warn_percent,
            memory_warn_percent: self.inspection.memory_warn_percent,
        })
    }
}

fn default_true() -> bool {
    true
}

fn default_alert_send_resolved() -> bool {
    true
}

fn default_alert_smtp_port() -> u16 {
    587
}

fn default_alert_smtp_transport() -> AlertSmtpTransport {
    AlertSmtpTransport::StartTls
}

fn default_alert_scope_mode() -> AlertScopeMode {
    AlertScopeMode::All
}

fn default_inspection_delivery() -> Vec<AlertChannel> {
    vec![AlertChannel::Smtp]
}

fn dedup_alert_channels(values: Vec<AlertChannel>) -> Vec<AlertChannel> {
    let mut deduped = Vec::new();
    for value in values {
        if deduped.contains(&value) {
            continue;
        }
        deduped.push(value);
    }
    deduped
}

fn normalize_optional_trimmed(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn normalize_node_ids(field: &str, values: &[String]) -> Result<Vec<String>, ConfigError> {
    let values = normalize_string_list(values.to_vec());
    for (index, value) in values.iter().enumerate() {
        validate_identifier(&format!("{field}[{index}]"), value)?;
    }
    Ok(values)
}

fn validate_email_address(field: &str, value: &str) -> Result<(), ConfigError> {
    validate_non_empty(field, value)?;
    if !value.contains('@') || value.starts_with('@') || value.ends_with('@') {
        return Err(ConfigError::new(format!(
            "{field} must look like an email address"
        )));
    }
    Ok(())
}

fn validate_local_time(field: &str, value: &str) -> Result<(), ConfigError> {
    let trimmed = value.trim();
    let mut parts = trimmed.split(':');
    let (Some(hours), Some(minutes), None) = (parts.next(), parts.next(), parts.next()) else {
        return Err(ConfigError::new(format!("{field} must use HH:MM format")));
    };
    let hours = hours
        .parse::<u8>()
        .map_err(|_| ConfigError::new(format!("{field} must use HH:MM format")))?;
    let minutes = minutes
        .parse::<u8>()
        .map_err(|_| ConfigError::new(format!("{field} must use HH:MM format")))?;
    if hours > 23 || minutes > 59 {
        return Err(ConfigError::new(format!("{field} must use HH:MM format")));
    }
    Ok(())
}
