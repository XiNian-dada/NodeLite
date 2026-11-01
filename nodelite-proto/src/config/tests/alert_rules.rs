//! Alert rule checks: uniqueness, metric ranges and duration ceilings.

use super::range_rules::{SERVER_SAMPLE, override_line, server_error, server_error_for};

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

#[test]
fn rejects_duplicate_rule_ids() {
    let error = server_error_for(&with_extra_rule("node-offline", "offline_minutes", 5));
    assert!(error.contains("alerts.rules["), "{error}");
    assert!(error.contains(".id duplicates"), "{error}");
}

#[test]
fn rejects_percentage_thresholds_above_100() {
    for metric in [
        "cpu_usage_percent",
        "memory_usage_percent",
        "disk_usage_percent",
        "traffic_usage_percent",
    ] {
        let error = server_error_for(&with_extra_rule("extra-rule", metric, 101));
        assert!(error.contains("].threshold"), "{metric}: {error}");
    }
    assert_parses(&with_extra_rule("extra-rule", "cpu_usage_percent", 100));
    assert_parses(&with_extra_rule("extra-rule", "latency_ms", 5000));
}

fn assert_parses(input: &str) {
    super::super::parse_server_config(input).expect("in-range rule should parse");
}

#[test]
fn rejects_oversized_rule_durations() {
    assert!(
        server_error("window_minutes", "20161").contains("alerts.rules[0].window_minutes"),
        "window ceiling follows history retention"
    );
    assert!(server_error("cooldown_minutes", "43201").contains("alerts.rules[0].cooldown_minutes"));
    assert!(
        server_error("cooldown_minutes", &u64::MAX.to_string())
            .contains("alerts.rules[0].cooldown_minutes")
    );
    assert_parses(&override_line(SERVER_SAMPLE, "window_minutes", "20160"));
    assert_parses(&override_line(SERVER_SAMPLE, "cooldown_minutes", "43200"));
}

#[test]
fn rejects_oversized_inspection_durations() {
    assert!(server_error("lookback_hours", "337").contains("alerts.inspection.lookback_hours"));
    assert!(
        server_error("offline_grace_minutes", "20161")
            .contains("alerts.inspection.offline_grace_minutes")
    );
}
