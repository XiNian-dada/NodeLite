//! 设置页 handlers:查询只读状态、修改认证设置、查看更新日志与触发敏感运维动作。
//!
//! 这里保留共享类型与导出,把读路径、认证变更、更新相关流程拆到独立子模块,
//! 避免继续膨胀成一个数百行的 monolith。

mod config_edit;
mod helpers;
mod query;
mod security;
mod types;
mod updates;

pub(crate) use query::settings;
pub(crate) use security::{
    change_readonly_password, disable_two_factor, enable_two_factor, start_two_factor_setup,
};
pub(crate) use updates::{refresh_node_token, server_update_log, start_server_update};

use config_edit::{persist_auth_2fa_change, persist_auth_password_change};
use helpers::{
    generate_totp_secret, otpauth_uri, server_build_version, server_update_cache_dir,
    server_update_log_path, server_update_shell_command, server_update_writable_paths,
    settings_json_error, validate_password_for_settings,
};
use types::{
    ChangePasswordRequest, DisableTwoFactorRequest, EnableTwoFactorRequest,
    NodeTokenRefreshResponse, ServerUpdateLogQuery, ServerUpdateLogResponse,
    SettingsActionResponse, SettingsAgentToken, SettingsAuth, SettingsResponse, SettingsUpdates,
    StartServerUpdateRequest, TwoFactorSetupResponse,
};

pub(super) const MAX_UPDATE_LOG_CHUNK_BYTES: u64 = 128 * 1024;
