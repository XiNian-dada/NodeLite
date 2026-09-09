//! Agent configuration parsing and validation tests.

use super::super::{MAX_NODE_IDENTITY_TEXT_BYTES, MAX_NODE_TAG_BYTES, parse_agent_config};

#[test]
fn rejects_invalid_agent_server_scheme() {
    let error = parse_agent_config(
        r#"
        [agent]
        node_id = "hk-01"
        node_label = "Hong Kong 01"
        server = "http://127.0.0.1:8080/ws"
        token = "token"
        "#,
    )
    .expect_err("invalid agent config should fail");

    assert!(error.to_string().contains("agent.server"));
}

#[test]
fn parses_agent_config() {
    let config = parse_agent_config(
        r#"
        [agent]
        node_id = "hk-01"
        node_label = "Hong Kong 01"
        server = "ws://127.0.0.1:8080/ws"
        token = "token"
        report_interval_secs = 7
        hostname_override = "hk-01.internal"
        tags = [" edge ", "apac"]
        "#,
    )
    .expect("agent config should parse");

    assert_eq!(config.node_id, "hk-01");
    assert_eq!(config.report_interval_secs, 7);
    assert_eq!(config.tags, vec!["apac", "edge"]);
    assert_eq!(config.auth_timeout_secs, 20);
    assert_eq!(config.send_timeout_secs, 20);
    assert_eq!(config.inbound_timeout_secs, 90);
}

#[test]
fn agent_transport_deadlines_are_configurable_and_bounded() {
    let sample = include_str!("../../../../config/agent.example.toml");
    let with_deadline = |name, seconds| {
        sample
            .lines()
            .map(|line| {
                if line.starts_with(&format!("{name} =")) {
                    format!("{name} = {seconds}")
                } else {
                    line.to_string()
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    for name in [
        "connect_timeout_secs",
        "auth_timeout_secs",
        "send_timeout_secs",
        "inbound_timeout_secs",
    ] {
        for seconds in [0, 3601, u64::MAX] {
            assert!(
                parse_agent_config(&with_deadline(name, seconds)).is_err(),
                "accepted {name}={seconds}"
            );
        }
        for seconds in [1, 3600] {
            let config = parse_agent_config(&with_deadline(name, seconds)).expect("valid deadline");
            let actual = match name {
                "connect_timeout_secs" => config.connect_timeout_secs,
                "auth_timeout_secs" => config.auth_timeout_secs,
                "send_timeout_secs" => config.send_timeout_secs,
                _ => config.inbound_timeout_secs,
            };
            assert_eq!(actual, seconds);
        }
    }
}

#[test]
fn rejects_agent_config_with_too_many_tags() {
    let tags = (0..1000)
        .map(|index| format!("\"tag-{index}\""))
        .collect::<Vec<_>>()
        .join(", ");
    let input = format!(
        r#"
        [agent]
        node_id = "hk-01"
        node_label = "Hong Kong 01"
        server = "ws://127.0.0.1:8080/ws"
        token = "token"
        tags = [{tags}]
        "#
    );

    let error = parse_agent_config(&input).expect_err("too many tags should fail");
    assert!(error.to_string().contains("agent.tags"));
}

#[test]
fn rejects_agent_config_with_oversized_tag() {
    let oversized = "x".repeat(MAX_NODE_TAG_BYTES + 1);
    let input = format!(
        r#"
        [agent]
        node_id = "hk-01"
        node_label = "Hong Kong 01"
        server = "ws://127.0.0.1:8080/ws"
        token = "token"
        tags = ["{oversized}"]
        "#
    );

    let error = parse_agent_config(&input).expect_err("oversized tag should fail");
    assert!(error.to_string().contains("agent.tags[0]"));
}

#[test]
fn rejects_agent_config_with_oversized_identity_text() {
    let oversized = "界".repeat(MAX_NODE_IDENTITY_TEXT_BYTES / 3 + 1);
    for (field, extra) in [
        ("agent.node_label", format!("node_label = \"{oversized}\"")),
        (
            "agent.hostname_override",
            format!("node_label = \"Hong Kong 01\"\nhostname_override = \"{oversized}\""),
        ),
    ] {
        let input = format!(
            r#"
            [agent]
            node_id = "hk-01"
            {extra}
            server = "ws://127.0.0.1:8080/ws"
            token = "token"
            "#
        );

        let error = parse_agent_config(&input).expect_err("oversized identity text should fail");
        assert!(error.to_string().contains(field));
    }
}
