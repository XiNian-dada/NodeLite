use nodelite_proto::{
    AlertChannel, AlertComparator, AlertMetric, AlertRuleConfig, AlertScopeMode, AlertSeverity,
    AlertSmtpConfig, AlertSmtpTransport, AlertWebhookConfig, AlertingConfig, InspectionConfig,
};

use super::{update_alerting_settings, update_auth_2fa, update_auth_password};

const AUTH_WITH_TRAILING_COMMENT: &str =
    include_str!("../../../tests/fixtures/config_edit/auth_with_trailing_comment.toml");
const AUTH_2FA_BASE: &str = include_str!("../../../tests/fixtures/config_edit/auth_2fa_base.toml");
const AUTH_2FA_ENABLED: &str =
    include_str!("../../../tests/fixtures/config_edit/auth_2fa_enabled.toml");
const MISSING_AUTH: &str = include_str!("../../../tests/fixtures/config_edit/missing_auth.toml");
const ALERTS_BASE: &str = include_str!("../../../tests/fixtures/config_edit/alerts_base.toml");
const ALERTS_MISSING: &str =
    include_str!("../../../tests/fixtures/config_edit/alerts_missing.toml");
const ALERTS_WITH_COMMENTS: &str =
    include_str!("../../../tests/fixtures/config_edit/alerts_with_comments.toml");
const ALERTS_RULES_BY_ID: &str =
    include_str!("../../../tests/fixtures/config_edit/alerts_rules_by_id.toml");

#[test]
fn update_auth_password_preserves_trailing_comment_and_multiline_neighbors() {
    let updated = update_auth_password(AUTH_WITH_TRAILING_COMMENT, "new-pass")
        .expect("password change should preserve neighboring TOML");

    assert!(updated.contains(r#"password = "new-pass" # keep this comment"#));
    assert!(updated.contains("welcome = \"\"\"\nhello\nworld\n\"\"\""));
}

#[test]
fn update_auth_2fa_enables_and_preserves_auth_section() {
    let updated = update_auth_2fa(AUTH_2FA_BASE, true, Some("JBSWY3DPEHPK3PXP"))
        .expect("2FA enable should update auth section");

    assert!(updated.contains("username = \"viewer\""));
    assert!(updated.contains("password = \"old-pass\""));
    assert!(updated.contains("enable_2fa = true"));
    assert!(updated.contains("totp_secret = \"JBSWY3DPEHPK3PXP\""));
    assert!(updated.contains("[ui]"));
}

#[test]
fn update_auth_2fa_disables_and_removes_stale_secret() {
    let updated = update_auth_2fa(AUTH_2FA_ENABLED, false, None)
        .expect("2FA disable should update auth section");

    assert!(updated.contains("enable_2fa = false"));
    assert!(!updated.contains("totp_secret"));
}

#[test]
fn update_auth_password_rejects_missing_auth_section() {
    let error =
        update_auth_password(MISSING_AUTH, "new-pass").expect_err("missing auth should fail");
    assert!(error.to_string().contains("[auth] section"));
}

#[test]
fn update_alerting_settings_replaces_alerts_section_and_preserves_other_sections() {
    let updated = update_alerting_settings(ALERTS_BASE, &sample_alerting_config())
        .expect("alert settings update should succeed");

    assert!(updated.contains("[alerts]"));
    assert!(updated.contains("host = \"smtp.example.com\""));
    assert!(updated.contains("[[alerts.rules]]"));
    assert!(updated.contains("metric = \"cpu_usage_percent\""));
    assert!(updated.contains("[auth]"));
    assert!(updated.contains("username = \"viewer\""));
}

#[test]
fn update_alerting_settings_creates_alerts_section_when_missing() {
    let updated = update_alerting_settings(ALERTS_MISSING, &sample_alerting_config())
        .expect("missing alerts section should be created");

    assert!(updated.contains("[alerts]"));
    assert!(updated.contains("enabled = true"));
    assert!(updated.contains("[alerts.smtp]"));
}

#[test]
fn update_alerting_settings_preserves_existing_alert_comments() {
    let updated = update_alerting_settings(ALERTS_WITH_COMMENTS, &sample_alerting_config())
        .expect("alert settings update should preserve existing comments");

    assert!(updated.contains(r#"enabled = true # keep enabled comment"#));
    assert!(updated.contains(r#"host = "smtp.example.com" # keep host comment"#));
}

#[test]
fn update_alerting_settings_preserves_rule_comments_by_id() {
    let mut alerting = sample_alerting_config();
    alerting.rules = vec![
        AlertRuleConfig {
            id: "memory-hot".to_string(),
            name: "Memory".to_string(),
            metric: AlertMetric::MemoryUsagePercent,
            threshold: 90,
            ..sample_rule("memory-hot")
        },
        AlertRuleConfig {
            id: "cpu-hot".to_string(),
            name: "CPU".to_string(),
            metric: AlertMetric::CpuUsagePercent,
            threshold: 85,
            ..sample_rule("cpu-hot")
        },
    ];

    let updated = update_alerting_settings(ALERTS_RULES_BY_ID, &alerting)
        .expect("alert settings update should preserve rule comments by id");

    assert!(updated.contains(r#"id = "memory-hot" # memory id comment"#));
    assert!(updated.contains(r#"id = "cpu-hot" # cpu id comment"#));
    assert!(updated.contains(r#"name = "Memory""#));
    assert!(updated.contains(r#"name = "CPU""#));
}

fn sample_alerting_config() -> AlertingConfig {
    AlertingConfig {
        enabled: true,
        smtp: AlertSmtpConfig {
            enabled: true,
            host: "smtp.example.com".to_string(),
            port: 587,
            username: "ops".to_string(),
            password: Some("smtp-secret".to_string()),
            sender: "nodelite@example.com".to_string(),
            recipients: vec!["ops@example.com".to_string()],
            transport: AlertSmtpTransport::StartTls,
            send_resolved: true,
        },
        webhook: AlertWebhookConfig {
            enabled: true,
            url: "https://hooks.example.com/nodelite".to_string(),
            secret: Some("hook-secret".to_string()),
            send_resolved: true,
        },
        rules: vec![AlertRuleConfig {
            id: "cpu-hot".to_string(),
            name: "CPU".to_string(),
            enabled: true,
            metric: AlertMetric::CpuUsagePercent,
            comparator: AlertComparator::Gt,
            threshold: 85,
            window_minutes: 5,
            severity: AlertSeverity::Critical,
            scope_mode: AlertScopeMode::All,
            node_ids: Vec::new(),
            tags: Vec::new(),
            delivery: vec![AlertChannel::Smtp],
            cooldown_minutes: 30,
            send_resolved: true,
        }],
        inspection: InspectionConfig::default(),
    }
}

fn sample_rule(id: &str) -> AlertRuleConfig {
    AlertRuleConfig {
        id: id.to_string(),
        name: "Rule".to_string(),
        enabled: true,
        metric: AlertMetric::CpuUsagePercent,
        comparator: AlertComparator::Gt,
        threshold: 85,
        window_minutes: 5,
        severity: AlertSeverity::Critical,
        scope_mode: AlertScopeMode::All,
        node_ids: Vec::new(),
        tags: Vec::new(),
        delivery: vec![AlertChannel::Smtp],
        cooldown_minutes: 30,
        send_resolved: true,
    }
}
