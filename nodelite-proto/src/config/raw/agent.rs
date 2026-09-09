//! Agent TOML decoding keeps transport deadlines finite and configurable.

use serde::Deserialize;

use super::super::defaults::{
    default_agent_auth_timeout_secs, default_agent_inbound_timeout_secs,
    default_agent_send_timeout_secs, default_connect_timeout_secs,
    default_insecure_transport_warn_interval_secs, default_max_incoming_message_bytes,
    default_report_interval_secs,
};
use super::super::helpers::{normalize_tags, validate_url};
use super::super::{AgentConfig, ConfigError, MAX_NODE_IDENTITY_TEXT_BYTES};
use crate::validation::{validate_bounded_text, validate_identifier, validate_non_empty};

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::config) struct RawAgentConfigFile {
    agent: RawAgentSection,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawAgentSection {
    node_id: String,
    node_label: String,
    server: String,
    token: String,
    #[serde(default = "default_report_interval_secs")]
    report_interval_secs: u64,
    hostname_override: Option<String>,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default = "default_connect_timeout_secs")]
    connect_timeout_secs: u64,
    #[serde(default = "default_agent_auth_timeout_secs")]
    auth_timeout_secs: u64,
    #[serde(default = "default_agent_send_timeout_secs")]
    send_timeout_secs: u64,
    #[serde(default = "default_agent_inbound_timeout_secs")]
    inbound_timeout_secs: u64,
    #[serde(default = "default_max_incoming_message_bytes")]
    max_incoming_message_bytes: usize,
    #[serde(default = "default_insecure_transport_warn_interval_secs")]
    insecure_transport_warn_interval_secs: u64,
}

impl RawAgentConfigFile {
    /// 校验 Agent 配置,并把 `agent.tags` 等字段规范化(去空白、去重、排序)。
    pub(in crate::config) fn validate(self) -> Result<AgentConfig, ConfigError> {
        validate_identifier("agent.node_id", &self.agent.node_id)?;
        validate_bounded_text(
            "agent.node_label",
            &self.agent.node_label,
            MAX_NODE_IDENTITY_TEXT_BYTES,
        )?;
        validate_url("agent.server", &self.agent.server, &["ws", "wss"])?;
        validate_non_empty("agent.token", &self.agent.token)?;

        if self.agent.report_interval_secs < 1 {
            return Err(ConfigError::new(
                "agent.report_interval_secs must be at least 1 second",
            ));
        }

        for (name, seconds) in [
            ("connect_timeout_secs", self.agent.connect_timeout_secs),
            ("auth_timeout_secs", self.agent.auth_timeout_secs),
            ("send_timeout_secs", self.agent.send_timeout_secs),
            ("inbound_timeout_secs", self.agent.inbound_timeout_secs),
        ] {
            if !(1..=3600).contains(&seconds) {
                return Err(ConfigError::new(format!(
                    "agent.{name} must be between 1 and 3600 seconds"
                )));
            }
        }

        if let Some(hostname) = &self.agent.hostname_override {
            validate_bounded_text(
                "agent.hostname_override",
                hostname,
                MAX_NODE_IDENTITY_TEXT_BYTES,
            )?;
        }

        Ok(AgentConfig {
            node_id: self.agent.node_id.trim().to_string(),
            node_label: self.agent.node_label.trim().to_string(),
            server: self.agent.server,
            token: self.agent.token,
            report_interval_secs: self.agent.report_interval_secs,
            hostname_override: self
                .agent
                .hostname_override
                .map(|value| value.trim().to_string()),
            tags: normalize_tags("agent.tags", self.agent.tags)?,
            connect_timeout_secs: self.agent.connect_timeout_secs,
            auth_timeout_secs: self.agent.auth_timeout_secs,
            send_timeout_secs: self.agent.send_timeout_secs,
            inbound_timeout_secs: self.agent.inbound_timeout_secs,
            max_incoming_message_bytes: self.agent.max_incoming_message_bytes,
            insecure_transport_warn_interval_secs: self.agent.insecure_transport_warn_interval_secs,
        })
    }
}
