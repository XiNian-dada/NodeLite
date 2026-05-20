use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// 设置页读取的服务端与安全状态。这里刻意不包含任何 token / password 明文。
#[derive(Debug, Serialize)]
pub(crate) struct SettingsResponse {
    pub(crate) service: &'static str,
    pub(crate) server_version: &'static str,
    pub(crate) repository: &'static str,
    pub(crate) public_base_url: String,
    pub(crate) listen: String,
    pub(crate) config_path: String,
    pub(crate) registry_path: String,
    pub(crate) history_db_path: String,
    pub(crate) snapshot_path: String,
    pub(crate) history_retention_hours: u64,
    pub(crate) refresh_interval_secs: u64,
    pub(crate) auth: SettingsAuth,
    pub(crate) updates: SettingsUpdates,
    pub(crate) agents: Vec<SettingsAgentToken>,
}

#[derive(Debug, Serialize)]
pub(crate) struct SettingsAuth {
    pub(crate) enabled: bool,
    pub(crate) username: Option<String>,
    pub(crate) two_factor_enabled: bool,
    pub(crate) totp_secret_configured: bool,
    pub(crate) session_ttl_secs: u64,
    pub(crate) pending_ttl_secs: u64,
}

#[derive(Debug, Serialize)]
pub(crate) struct SettingsUpdates {
    pub(crate) latest_release_url: String,
    pub(crate) server_upgrade_command: String,
    pub(crate) agent_upgrade_command: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct SettingsAgentToken {
    pub(crate) node_id: String,
    pub(crate) node_label: String,
    pub(crate) online: bool,
    pub(crate) agent_version: Option<String>,
    pub(crate) remote_ip: Option<String>,
    pub(crate) tags: Vec<String>,
    pub(crate) token_expires_at: Option<DateTime<Utc>>,
    pub(crate) token_expires_in_secs: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ChangePasswordRequest {
    pub(crate) current_password: String,
    pub(crate) new_password: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct StartServerUpdateRequest {
    pub(crate) current_password: Option<String>,
    pub(crate) code: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ServerUpdateLogQuery {
    pub(crate) offset: Option<u64>,
}

#[derive(Debug, Serialize)]
pub(crate) struct ServerUpdateLogResponse {
    pub(crate) exists: bool,
    pub(crate) offset: u64,
    pub(crate) next_offset: u64,
    pub(crate) text: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct TwoFactorSetupResponse {
    pub(crate) secret: String,
    pub(crate) otpauth_uri: String,
    pub(crate) qr_svg: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct EnableTwoFactorRequest {
    pub(crate) current_password: String,
    pub(crate) secret: String,
    pub(crate) code: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct DisableTwoFactorRequest {
    pub(crate) current_password: String,
    pub(crate) code: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct SettingsActionResponse {
    pub(crate) ok: bool,
    pub(crate) message: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct NodeTokenRefreshResponse {
    pub(crate) ok: bool,
    pub(crate) message: String,
    pub(crate) token_expires_at: Option<DateTime<Utc>>,
    pub(crate) token_expires_in_secs: Option<i64>,
}
