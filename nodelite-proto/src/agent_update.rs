//! Remote requests carry no execution authority; the node's root policy selects the signed target.

use serde::{Deserialize, Serialize};

pub const REQUEST_LIFETIME_SECS: u64 = 60;
pub const UPDATE_COOLDOWN_SECS: u64 = 300;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentUpdateRequest {
    pub request_id: String,
    pub issued_at: u64,
    pub expires_at: u64,
}

impl AgentUpdateRequest {
    pub fn validate(&self, now: u64) -> bool {
        valid_request_id(&self.request_id)
            && self.issued_at <= now.saturating_add(5)
            && self.expires_at > now
            && self.expires_at > self.issued_at
            && self.expires_at - self.issued_at <= REQUEST_LIFETIME_SECS
            && self.expires_at < 1 << 53
    }
}

pub fn valid_request_id(value: &str) -> bool {
    value.len() == 32
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentUpdateCapability {
    Ready,
    Disabled,
    Unsupported,
    ManualRecovery,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentUpdatePhase {
    Idle,
    Queued,
    Downloading,
    Prepared,
    Stopping,
    Trial,
    RollingBack,
    Committed,
    UpToDate,
    RolledBack,
    Failed,
    ManualRecovery,
    Unknown,
}

impl AgentUpdatePhase {
    pub fn in_progress(self) -> bool {
        matches!(
            self,
            Self::Queued
                | Self::Downloading
                | Self::Prepared
                | Self::Stopping
                | Self::Trial
                | Self::RollingBack
        )
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentUpdateReason {
    NotEnrolled,
    DisabledByPolicy,
    UnsupportedEnvironment,
    LocalStateInvalid,
    InvalidRequest,
    RateLimited,
    Busy,
    UpdateFailed,
    RequestExpired,
    ManualRecovery,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentUpdateStatus {
    pub capability: AgentUpdateCapability,
    pub phase: AgentUpdatePhase,
    pub request_id: Option<String>,
    pub updated_at: u64,
    pub installed_sequence: Option<u64>,
    pub target_sequence: Option<u64>,
    pub channel: Option<String>,
    pub reason: Option<AgentUpdateReason>,
}

impl AgentUpdateStatus {
    pub fn unavailable(capability: AgentUpdateCapability, reason: AgentUpdateReason) -> Self {
        Self {
            capability,
            reason: Some(reason),
            phase: AgentUpdatePhase::Idle,
            request_id: None,
            updated_at: 0,
            installed_sequence: None,
            target_sequence: None,
            channel: None,
        }
    }

    pub fn validate(&self) -> bool {
        self.request_id.as_deref().is_none_or(valid_request_id)
            && self.updated_at < 1 << 53
            && [self.installed_sequence, self.target_sequence]
                .into_iter()
                .flatten()
                .all(|sequence| sequence > 0 && sequence < 1 << 53)
            && self
                .channel
                .as_deref()
                .is_none_or(|channel| matches!(channel, "stable" | "testing"))
            && (!self.phase.in_progress() || self.request_id.is_some())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_reject_replay_windows_overflow_and_execution_fields() {
        let mut request = AgentUpdateRequest {
            request_id: "a".repeat(32),
            issued_at: 100,
            expires_at: 160,
        };
        assert!(request.validate(100));
        assert!(!request.validate(160));
        assert!(!request.validate(94));
        request.expires_at = u64::MAX;
        assert!(!request.validate(100));
        request.expires_at = 161;
        assert!(!request.validate(100));
        request.expires_at = 160;
        request.request_id = "../../etc/passwd".into();
        assert!(!request.validate(100));
        let data = r#"{"request_id":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","issued_at":100,"expires_at":160,"command":"sh"}"#;
        assert!(serde_json::from_str::<AgentUpdateRequest>(data).is_err());
    }

    #[test]
    fn status_is_bounded_and_only_known_channels_are_accepted() {
        let mut status = AgentUpdateStatus::unavailable(
            AgentUpdateCapability::Disabled,
            AgentUpdateReason::NotEnrolled,
        );
        assert!(status.validate());
        status.phase = AgentUpdatePhase::Downloading;
        assert!(!status.validate());
        status.request_id = Some("b".repeat(32));
        assert!(status.validate());
        status.channel = Some("https://evil.invalid/".into());
        assert!(!status.validate());
    }
}
