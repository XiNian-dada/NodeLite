//! Alert rule bounds: values saved by earlier versions clamp on load, strict on new writes.

use super::super::{
    AlertRuleConfig, ServerConfig, check_alerting_limits, parse_server_config_with_warnings,
};
use super::range_rules::{SERVER_SAMPLE, override_line};

fn with_extra_rule(id: &str, metric: &str, threshold: u64) -> String {
    format!(
        r#"{SERVER_SAMPLE}

[[alerts.rules]]
id = "{id}"
name = "extra"
metric = "{metric}"
comparator = "gt"
threshold = {threshold}
severity = "warning"
"#
    )
}

fn parse_with_warnings(input: &str) -> (ServerConfig, Vec<String>) {
    parse_server_config_with_warnings(input).expect("existing out-of-range values should load")
}

fn last_rule(config: &ServerConfig) -> &AlertRuleConfig {
    config
        .alerting
        .rules
        .last()
        .expect("sample should define rules")
}

#[test]
fn duplicate_rule_ids_load_with_warning_but_fail_strict_check() {
    let (config, warnings) =
        parse_with_warnings(&with_extra_rule("node-offline", "offline_minutes", 5));
    assert!(
        warnings.iter().any(|w| w.contains("].id duplicates")),
        "{warnings:?}"
    );
    let error = check_alerting_limits(&config.alerting).expect_err("strict check rejects dups");
    assert!(error.to_string().contains("alerts.rules["));
}

#[test]
fn percentage_thresholds_above_100_clamp_on_load_and_fail_strict_check() {
    for metric in [
        "cpu_usage_percent",
        "memory_usage_percent",
        "disk_usage_percent",
    ] {
        let (config, warnings) = parse_with_warnings(&with_extra_rule("extra-rule", metric, 150));
        assert_eq!(last_rule(&config).threshold, 100, "{metric}");
        assert!(
            warnings.iter().any(|w| w.contains("].threshold")),
            "{metric}"
        );
        check_alerting_limits(&config.alerting).expect("clamped config passes strict check");

        let mut raw = config.alerting.clone();
        if let Some(rule) = raw.rules.last_mut() {
            rule.threshold = 101;
        }
        let error = check_alerting_limits(&raw).expect_err("new writes above 100 fail");
        assert!(error.to_string().contains("].threshold"), "{metric}");
    }
}

#[test]
fn traffic_usage_above_100_percent_is_valid() {
    let (config, warnings) = parse_with_warnings(&with_extra_rule(
        "traffic-overage",
        "traffic_usage_percent",
        120,
    ));
    assert_eq!(last_rule(&config).threshold, 120);
    assert!(warnings.is_empty(), "{warnings:?}");
    check_alerting_limits(&config.alerting).expect("120% traffic rule is a valid write");
}

#[test]
fn oversized_rule_durations_clamp_on_load() {
    let input = override_line(SERVER_SAMPLE, "window_minutes", "20161");
    let input = override_line(&input, "cooldown_minutes", "99999999");
    let (config, warnings) = parse_with_warnings(&input);
    assert_eq!(config.alerting.rules[0].window_minutes, 20160);
    assert_eq!(config.alerting.rules[0].cooldown_minutes, 43200);
    assert!(
        warnings
            .iter()
            .any(|w| w.contains("alerts.rules[0].window_minutes"))
    );
    assert!(
        warnings
            .iter()
            .any(|w| w.contains("alerts.rules[0].cooldown_minutes"))
    );

    let mut raw = config.alerting.clone();
    raw.rules[0].cooldown_minutes = 43201;
    let error = check_alerting_limits(&raw).expect_err("new writes above the cap fail");
    assert!(
        error
            .to_string()
            .contains("alerts.rules[0].cooldown_minutes")
    );
}

#[test]
fn oversized_inspection_durations_clamp_on_load() {
    let input = override_line(SERVER_SAMPLE, "lookback_hours", "720");
    let input = override_line(&input, "offline_grace_minutes", "20161");
    let (config, warnings) = parse_with_warnings(&input);
    assert_eq!(config.alerting.inspection.lookback_hours, 336);
    assert_eq!(config.alerting.inspection.offline_grace_minutes, 20160);
    assert!(
        warnings
            .iter()
            .any(|w| w.contains("alerts.inspection.lookback_hours"))
    );
    assert!(
        warnings
            .iter()
            .any(|w| w.contains("alerts.inspection.offline_grace_minutes"))
    );

    let mut raw = config.alerting.clone();
    raw.inspection.lookback_hours = 337;
    let error = check_alerting_limits(&raw).expect_err("new writes above 336 fail");
    assert!(
        error
            .to_string()
            .contains("alerts.inspection.lookback_hours")
    );
}

#[test]
fn shipped_example_loads_without_warnings() {
    let (config, warnings) = parse_with_warnings(SERVER_SAMPLE);
    assert!(warnings.is_empty(), "{warnings:?}");
    check_alerting_limits(&config.alerting).expect("example alerting passes strict check");
}
