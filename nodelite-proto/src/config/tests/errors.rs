//! Diagnostics must identify invalid configuration without retaining its values or source text.

use super::super::{ConfigError, parse_agent_config};

fn assert_redacted(error: &ConfigError, marker: &str) {
    assert!(
        !error.to_string().contains(marker),
        "display exposed an input value"
    );
    assert!(
        !format!("{error:?}").contains(marker),
        "debug exposed an input value"
    );
    assert!(std::error::Error::source(error).is_none());
}

#[test]
fn agent_parse_diagnostics_do_not_retain_values() {
    let marker = "synthetic-agent-value";
    let input = format!(
        "[agent]\nnode_id = 'fixture'\nnode_label = 'Fixture'\nserver = 'ws://127.0.0.1/ws'\ntoken = '{marker}\n"
    );
    let error = parse_agent_config(&input).expect_err("invalid configuration must be rejected");
    assert_redacted(&error, marker);
    assert!(error.to_string().contains("line"));
    assert!(error.to_string().contains("column"));
}

#[cfg(feature = "server-config")]
#[test]
fn server_parse_diagnostics_do_not_retain_credential_fields() {
    let base = "[server]\nlisten = '127.0.0.1:8080'\npublic_base_url = 'https://example.invalid'\n";
    for (section, field) in [
        ("auth", "password"),
        ("auth", "totp_secret"),
        ("alerts.smtp", "password"),
        ("alerts.webhook", "secret"),
        ("alerts.webhook", "url"),
    ] {
        let marker = "synthetic-credential-value";
        let input = format!("{base}\n[{section}]\n{field} = '{marker}\n");
        let error = super::super::parse_server_config(&input)
            .expect_err("invalid configuration must be rejected");
        assert_redacted(&error, marker);
        assert!(error.to_string().contains("line"));
        assert!(error.to_string().contains("column"));
    }
}

#[cfg(feature = "server-config")]
#[test]
fn server_schema_diagnostics_do_not_echo_values_keys_or_variants() {
    let base = "[server]\nlisten = '127.0.0.1:8080'\npublic_base_url = 'https://example.invalid'\n";
    let marker = "synthetic-schema-value";
    for section in [
        format!("[auth]\nusername = 'viewer'\npassword = '{marker}'\nenable_2fa = '{marker}'\n"),
        format!("[alerts.smtp]\ntransport = '{marker}'\n"),
        format!("[geoip]\n{marker} = true\n"),
        format!("['{marker}']\nenabled = true\n"),
    ] {
        let input = format!("{base}\n{section}");
        let error = super::super::parse_server_config(&input)
            .expect_err("invalid configuration must be rejected");
        assert_redacted(&error, marker);
    }
}

#[cfg(feature = "server-config")]
#[test]
fn invalid_proxy_diagnostics_do_not_echo_the_input() {
    let marker = "synthetic-proxy-value";
    let input = format!(
        "[server]\nlisten = '127.0.0.1:8080'\npublic_base_url = 'https://example.invalid'\ntrusted_proxies = ['{marker}']\n"
    );
    let error = super::super::parse_server_config(&input)
        .expect_err("invalid configuration must be rejected");
    assert_redacted(&error, marker);
    assert!(error.to_string().contains("server.trusted_proxies"));
}

#[test]
fn toml_diagnostics_use_only_fixed_categories_and_bounded_byte_locations() {
    let input = "秘密\r\nx = 1\n";
    for (offset, position) in [
        (0, "line 1, column 1"),
        (2, "line 1, column 3"),
        (8, "line 2, column 1"),
        (usize::MAX, "line 3, column 1"),
    ] {
        let error = ConfigError::from_toml_diagnostic(
            input,
            "unknown field `synthetic`",
            Some(offset..offset),
        );
        assert!(error.to_string().contains(position));
        assert!(error.to_string().contains("unknown field"));
        assert_redacted(&error, "synthetic");
        assert_redacted(&error, "秘密");
    }
    for message in [
        "synthetic",
        "invalid value: synthetic",
        "duplicate key: synthetic",
        "unknown variant synthetic",
        "missing field synthetic",
        "invalid type: synthetic",
    ] {
        let error = ConfigError::from_toml_diagnostic("", message, None);
        assert!(error.to_string().starts_with("invalid TOML configuration:"));
        assert_redacted(&error, "synthetic");
    }
}
