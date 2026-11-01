//! Range checks that keep zero or oversized timers from surfacing as runtime panics.

use super::super::parse_agent_config;
#[cfg(feature = "server-config")]
use super::super::parse_server_config;

/// 只替换示例配置中第一处 `name = ...`，保证每个用例只改动一个字段。
pub(super) fn override_line(sample: &str, name: &str, value: &str) -> String {
    let prefix = format!("{name} =");
    let mut replaced = false;
    let lines = sample
        .lines()
        .map(|line| {
            if !replaced && line.trim_start().starts_with(&prefix) {
                replaced = true;
                format!("{name} = {value}")
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(replaced, "sample should contain {name}");
    lines
}

const AGENT_SAMPLE: &str = include_str!("../../../../config/agent.example.toml");

fn agent_error(name: &str, value: &str) -> String {
    parse_agent_config(&override_line(AGENT_SAMPLE, name, value))
        .expect_err("out-of-range agent value should fail")
        .to_string()
}

#[test]
fn shipped_agent_example_still_parses() {
    parse_agent_config(AGENT_SAMPLE).expect("agent example should parse");
}

#[test]
fn agent_rejects_tiny_incoming_message_limit() {
    for value in ["0", "1023"] {
        assert!(
            agent_error("max_incoming_message_bytes", value)
                .contains("agent.max_incoming_message_bytes")
        );
    }
    parse_agent_config(&override_line(
        AGENT_SAMPLE,
        "max_incoming_message_bytes",
        "1024",
    ))
    .expect("the floor itself should parse");
}

#[test]
fn agent_rejects_zero_or_huge_warn_interval() {
    for value in ["0", "86401"] {
        assert!(
            agent_error("insecure_transport_warn_interval_secs", value)
                .contains("agent.insecure_transport_warn_interval_secs")
        );
    }
}

#[cfg(feature = "server-config")]
pub(super) const SERVER_SAMPLE: &str = include_str!("../../../../config/server.example.toml");

#[cfg(feature = "server-config")]
pub(super) fn server_error_for(input: &str) -> String {
    parse_server_config(input)
        .expect_err("invalid server value should fail")
        .to_string()
}

#[cfg(feature = "server-config")]
pub(super) fn server_error(name: &str, value: &str) -> String {
    server_error_for(&override_line(SERVER_SAMPLE, name, value))
}

#[cfg(feature = "server-config")]
#[test]
fn shipped_server_example_still_parses() {
    parse_server_config(SERVER_SAMPLE).expect("server example should parse");
}

#[cfg(feature = "server-config")]
#[test]
fn server_rejects_zero_or_huge_hello_timeout() {
    for value in ["0", "301"] {
        assert!(server_error("hello_timeout_secs", value).contains("server.hello_timeout_secs"));
    }
}

#[cfg(feature = "server-config")]
#[test]
fn server_rejects_zero_or_huge_warn_interval() {
    for value in ["0", "86401"] {
        assert!(
            server_error("insecure_transport_warn_interval_secs", value)
                .contains("server.insecure_transport_warn_interval_secs")
        );
    }
}
