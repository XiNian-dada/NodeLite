//! 审计事件载荷、查询过滤与错误类型定义。

use anyhow::Error;
use chrono::{DateTime, Utc};
use nodelite_proto::truncate_string_to_byte_boundary;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// Rows stay for the whole retention period, so their size must not follow request size.
const MAX_AUDIT_TEXT_BYTES: usize = 512;
const MAX_AUDIT_DETAILS_BYTES: usize = 4096;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AuditEventType {
    AgentUpdateRequested,
    LoginSuccess,
    LoginFailure,
    TotpVerifySuccess,
    TotpVerifyFailure,
    PasskeyVerifySuccess,
    PasskeyVerifyFailure,
    NodeConnected,
    TokenInvalid,
    RateLimitExceeded,
}

impl AuditEventType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::AgentUpdateRequested => "agent_update_requested",
            Self::LoginSuccess => "login_success",
            Self::LoginFailure => "login_failure",
            Self::TotpVerifySuccess => "totp_verify_success",
            Self::TotpVerifyFailure => "totp_verify_failure",
            Self::PasskeyVerifySuccess => "passkey_verify_success",
            Self::PasskeyVerifyFailure => "passkey_verify_failure",
            Self::NodeConnected => "node_connected",
            Self::TokenInvalid => "token_invalid",
            Self::RateLimitExceeded => "rate_limit_exceeded",
        }
    }

    pub fn parse(input: &str) -> Option<Self> {
        match input {
            "agent_update_requested" => Some(Self::AgentUpdateRequested),
            "login_success" => Some(Self::LoginSuccess),
            "login_failure" => Some(Self::LoginFailure),
            "totp_verify_success" => Some(Self::TotpVerifySuccess),
            "totp_verify_failure" => Some(Self::TotpVerifyFailure),
            "passkey_verify_success" => Some(Self::PasskeyVerifySuccess),
            "passkey_verify_failure" => Some(Self::PasskeyVerifyFailure),
            "node_connected" => Some(Self::NodeConnected),
            "token_invalid" => Some(Self::TokenInvalid),
            "rate_limit_exceeded" => Some(Self::RateLimitExceeded),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AuditEvent {
    pub id: i64,
    pub timestamp: DateTime<Utc>,
    pub event_type: AuditEventType,
    pub user: Option<String>,
    pub node_id: Option<String>,
    pub ip_address: String,
    pub user_agent: Option<String>,
    pub success: bool,
    pub details: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NewAuditEvent {
    pub timestamp: DateTime<Utc>,
    pub event_type: AuditEventType,
    pub user: Option<String>,
    pub node_id: Option<String>,
    pub ip_address: String,
    pub user_agent: Option<String>,
    pub success: bool,
    pub details: Value,
}

impl NewAuditEvent {
    pub fn now(event_type: AuditEventType, ip_address: impl Into<String>, success: bool) -> Self {
        Self {
            timestamp: Utc::now(),
            event_type,
            user: None,
            node_id: None,
            ip_address: ip_address.into(),
            user_agent: None,
            success,
            details: Value::Object(Default::default()),
        }
    }

    /// Text fields can come straight from a client, so every write path stores this form.
    pub(super) fn bounded(mut self) -> Self {
        self.user = self.user.map(bounded_text);
        self.node_id = self.node_id.map(bounded_text);
        self.user_agent = self.user_agent.map(bounded_text);
        self.ip_address = bounded_text(self.ip_address);
        bound_detail_strings(&mut self.details);
        if self.details.to_string().len() > MAX_AUDIT_DETAILS_BYTES {
            self.details = json!({ "truncated": true });
        }
        self
    }
}

fn bounded_text(value: String) -> String {
    let mut text: String = value.chars().filter(|c| !c.is_control()).collect();
    truncate_string_to_byte_boundary(&mut text, MAX_AUDIT_TEXT_BYTES);
    text
}

/// One long value must not push short ones such as `reason` out of the stored details.
fn bound_detail_strings(value: &mut Value) {
    match value {
        Value::String(text) => truncate_string_to_byte_boundary(text, MAX_AUDIT_TEXT_BYTES),
        Value::Array(items) => items.iter_mut().for_each(bound_detail_strings),
        Value::Object(fields) => fields.values_mut().for_each(bound_detail_strings),
        _ => {}
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct AuditQuery {
    pub start: Option<DateTime<Utc>>,
    pub end: Option<DateTime<Utc>>,
    pub event_type: Option<AuditEventType>,
    pub success: Option<bool>,
    pub limit: usize,
}

#[derive(Debug)]
pub enum AuditLogError {
    Disabled,
    Query(Error),
}

impl std::fmt::Display for AuditLogError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Disabled => f.write_str("audit log is disabled"),
            Self::Query(_) => f.write_str("audit log query failed"),
        }
    }
}

impl std::error::Error for AuditLogError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Query(error) => Some(error.root_cause()),
            Self::Disabled => None,
        }
    }
}
