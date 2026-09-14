//! Optional sections must share defaults without relaxing required or unknown fields.

use super::super::parse_server_config;

const BASE: &str =
    "[server]\nlisten = '127.0.0.1:8080'\npublic_base_url = 'http://127.0.0.1:8080'\n";

#[test]
fn absent_and_empty_sections_have_identical_defaults() {
    let expected = parse_server_config(BASE).expect("minimal config");
    for section in [
        "ui",
        "ws",
        "metrics",
        "audit",
        "agent_logs",
        "geoip",
        "filters",
        "alerts.smtp",
        "alerts.webhook",
        "alerts.inspection",
    ] {
        let actual =
            parse_server_config(&format!("{BASE}\n[{section}]\n")).expect("empty optional section");
        assert_eq!(actual, expected, "section {section}");
        let error = parse_server_config(&format!("{BASE}\n[{section}]\nunknown_setting = 1\n"))
            .expect_err("unknown fields must be rejected");
        assert!(
            error.to_string().contains("unknown field"),
            "section {section}"
        );
    }
}

#[test]
fn partial_sections_keep_defaults_and_required_fields_stay_required() {
    let mut expected = parse_server_config(BASE).expect("minimal config");
    expected.audit.enabled = false;
    expected.ws.max_total_connections = 512;
    expected.alerting.smtp.port = 465;
    let actual = parse_server_config(&format!("{BASE}\n[audit]\nenabled = false\n[ws]\nmax_total_connections = 512\n[alerts.smtp]\nport = 465\n"))
        .expect("partial sections");
    assert_eq!(actual, expected);
    for input in ["", "[server]", "[server]\nlisten = '127.0.0.1:8080'"] {
        assert!(parse_server_config(input).is_err());
    }
}

#[test]
fn checked_in_server_example_parses() {
    parse_server_config(include_str!("../../../../config/server.example.toml"))
        .expect("server example must remain loadable");
}

#[test]
fn agent_log_budgets_have_bounded_default_and_low_memory_profiles() {
    let defaults = parse_server_config(BASE).expect("defaults").agent_logs;
    assert_eq!(defaults.max_entries, 10_000);
    assert_eq!(defaults.max_estimated_bytes, 8 * 1024 * 1024);
    for bytes in [65536, 2 * 1024 * 1024, 64 * 1024 * 1024] {
        let config = parse_server_config(&format!(
            "{BASE}\n[agent_logs]\nmax_entries = 128\nmax_estimated_bytes = {bytes}"
        ))
        .expect("bounded log budget");
        assert_eq!(config.agent_logs.max_estimated_bytes, bytes);
    }
    for (field, value) in [
        ("max_entries", 127),
        ("max_entries", 100001),
        ("max_estimated_bytes", 65535),
        ("max_estimated_bytes", 67108865),
    ] {
        let error = parse_server_config(&format!("{BASE}\n[agent_logs]\n{field} = {value}"))
            .expect_err("out-of-range log budget");
        assert!(error.to_string().contains(field));
    }
}
