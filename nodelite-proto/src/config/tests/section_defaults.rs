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
