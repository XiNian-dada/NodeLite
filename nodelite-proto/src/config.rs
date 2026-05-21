//! 配置文件解析:Agent 与 Server 启动时读取的 TOML 配置。
//!
//! 设计要点:
//! 1. 暴露的 [`ServerConfig`]/[`AgentConfig`] 是经过校验的"干净"结构。
//! 2. 原始 TOML 反序列化、默认值与内部校验 helper 分拆到子模块中,保持公开 API 稳定。
//! 3. 所有默认值通过常量 `DEFAULT_*` 暴露,供本模块与外部组件共享。

mod defaults;
mod helpers;
mod raw;
#[cfg(test)]
mod tests;

use std::net::SocketAddr;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use self::defaults::{
    default_connect_timeout_secs, default_hello_timeout_secs,
    default_insecure_transport_warn_interval_secs, default_max_incoming_message_bytes,
    default_max_outstanding_pings, default_max_sanitized_disks, default_max_sanitized_string_bytes,
    default_metric_anomaly_session_limit, default_sqlite_busy_timeout_secs,
};
use self::raw::{RawAgentConfigFile, RawServerConfigFile};

pub use self::helpers::normalize_totp_secret;

/// 节点超时阈值:超过该时长未收到任何报文即视为离线。
pub const DEFAULT_STALE_AFTER_SECS: u64 = 20;
/// Server 默认 ping 间隔(秒)。
pub const DEFAULT_PING_INTERVAL_SECS: u64 = 10;
/// WebSocket 单帧最大字节数,用于抑制恶意大包。
pub const DEFAULT_MAX_MESSAGE_BYTES: usize = 64 * 1024;
/// 前端默认刷新间隔(秒)。
pub const DEFAULT_REFRESH_INTERVAL_SECS: u64 = 5;
/// Agent 默认上报间隔(秒)。
pub const DEFAULT_REPORT_INTERVAL_SECS: u64 = 5;
/// 历史数据保留时长(小时),默认 14 天。
pub const DEFAULT_HISTORY_RETENTION_HOURS: u64 = 24 * 14;
/// 同一节点两次历史写入的最小间隔(秒),降低 SQLite 压力。
pub const DEFAULT_HISTORY_WRITE_INTERVAL_SECS: u64 = 30;
/// WebSocket 并发连接总数上限。
pub const DEFAULT_WS_MAX_TOTAL_CONNECTIONS: usize = 1024;
/// 单个 IP 允许的 WebSocket 并发连接数。
pub const DEFAULT_WS_MAX_CONNECTIONS_PER_IP: usize = 32;
/// 认证失败统计窗口(秒);超出该窗口的失败记录会被丢弃。
pub const DEFAULT_WS_AUTH_FAIL_WINDOW_SECS: u64 = 300;
/// 在统计窗口内允许的最大失败次数,达到后触发临时封禁。
pub const DEFAULT_WS_AUTH_FAIL_MAX_ATTEMPTS: usize = 12;
/// 触发封禁后的禁用时长(秒)。
pub const DEFAULT_WS_AUTH_BLOCK_SECS: u64 = 900;
/// 单个节点允许携带的最大标签数。
pub const MAX_NODE_TAGS: usize = 64;
/// 单个标签允许的最大字节数。
pub const MAX_NODE_TAG_BYTES: usize = 256;
/// WebSocket Hello 握手超时(秒)。
pub const DEFAULT_HELLO_TIMEOUT_SECS: u64 = 10;
/// 最大未响应 Ping 数量。
pub const DEFAULT_MAX_OUTSTANDING_PINGS: usize = 32;
/// 不安全传输警告间隔(秒)。
pub const DEFAULT_INSECURE_TRANSPORT_WARN_INTERVAL_SECS: u64 = 900;
/// 最大磁盘数量限制。
pub const DEFAULT_MAX_SANITIZED_DISKS: usize = 64;
/// 最大字符串字节数限制。
pub const DEFAULT_MAX_SANITIZED_STRING_BYTES: usize = 256;
/// 指标异常会话限制。
pub const DEFAULT_METRIC_ANOMALY_SESSION_LIMIT: usize = 5;
/// SQLite 忙等待超时(秒)。
pub const DEFAULT_SQLITE_BUSY_TIMEOUT_SECS: u64 = 5;
/// 审计日志默认保留天数。
pub const DEFAULT_AUDIT_RETENTION_DAYS: u64 = 90;
/// Agent 连接超时(秒)。
pub const DEFAULT_CONNECT_TIMEOUT_SECS: u64 = 20;
/// Agent 最大接收消息字节数。
pub const DEFAULT_MAX_INCOMING_MESSAGE_BYTES: usize = 64 * 1024;

/// 配置加载或校验过程中产生的错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigError {
    message: String,
}

impl ConfigError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ConfigError {}

/// Server 启动需要的全部配置。
///
/// #98: **故意不派生 `Serialize`**。`ServerConfig` 持有 `readonly_auth.password`
/// 与 `readonly_auth.totp_secret`,如果允许把整个结构直接序列化(`Json(config)`
/// / `serde_json::to_string(&config)`),任何一处疏忽就会让明文凭证泄露到响应、
/// 日志或调试输出。需要对外暴露字段时,请在 handler 内手工构造一个不带敏感字段
/// 的视图类型(参考 `handlers/settings/mod.rs::SettingsResponse`)。
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct ServerConfig {
    pub listen: SocketAddr,
    pub public_base_url: String,
    pub insecure_allow_http: bool,
    pub readonly_auth: Option<ReadonlyAuthConfig>,
    pub ws: WsConfig,
    pub audit: AuditConfig,
    pub node_registry_path: PathBuf,
    pub history_db_path: PathBuf,
    pub snapshot_path: PathBuf,
    pub stale_after_secs: u64,
    pub ping_interval_secs: u64,
    pub max_message_bytes: usize,
    pub refresh_interval_secs: u64,
    pub ignored_filesystems: Vec<String>,
    pub agent_release_base_url: Option<String>,
    pub agent_release_sha256_x86_64: Option<String>,
    pub agent_release_sha256_aarch64: Option<String>,
    #[serde(default = "default_hello_timeout_secs")]
    pub hello_timeout_secs: u64,
    #[serde(default = "default_max_outstanding_pings")]
    pub max_outstanding_pings: usize,
    #[serde(default = "default_insecure_transport_warn_interval_secs")]
    pub insecure_transport_warn_interval_secs: u64,
    #[serde(default = "default_max_sanitized_disks")]
    pub max_sanitized_disks: usize,
    #[serde(default = "default_max_sanitized_string_bytes")]
    pub max_sanitized_string_bytes: usize,
    #[serde(default = "default_metric_anomaly_session_limit")]
    pub metric_anomaly_session_limit: usize,
    #[serde(default = "default_sqlite_busy_timeout_secs")]
    pub sqlite_busy_timeout_secs: u64,
}

/// 前端只读访问所用的基本认证凭证。
///
/// #98: **故意不派生 `Serialize`**。`password` / `totp_secret` 是高敏字段,
/// 任何序列化路径(调试 `Json(auth_config)`、错误响应里 fmt-debug、
/// 自动派生 of 上层包装类型)都会直接泄露明文凭证。如果某个 handler 真的需要
/// 对前端公开一些非敏感子集(比如 username + enable_2fa),请显式定义一个视图
/// 结构 (`pub struct AuthPublicView { username: String, enable_2fa: bool }`)
/// 并只把它派生 `Serialize`。
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct ReadonlyAuthConfig {
    pub username: String,
    pub password: String,
    #[serde(default)]
    pub enable_2fa: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub totp_secret: Option<String>,
}

/// WebSocket 准入控制参数,用于限流与抗暴力破解。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WsConfig {
    pub max_total_connections: usize,
    pub max_connections_per_ip: usize,
    pub auth_fail_window_secs: u64,
    pub auth_fail_max_attempts: usize,
    pub auth_block_secs: u64,
}

/// 审计日志存储与记录策略。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AuditConfig {
    pub enabled: bool,
    pub db_path: PathBuf,
    pub retention_days: u64,
    pub log_successful_auth: bool,
    pub log_failed_auth: bool,
    pub log_token_events: bool,
    pub log_rate_limit: bool,
}

/// Agent 启动需要的全部配置。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentConfig {
    pub node_id: String,
    pub node_label: String,
    pub server: String,
    pub token: String,
    pub report_interval_secs: u64,
    pub hostname_override: Option<String>,
    pub tags: Vec<String>,
    #[serde(default = "default_connect_timeout_secs")]
    pub connect_timeout_secs: u64,
    #[serde(default = "default_max_incoming_message_bytes")]
    pub max_incoming_message_bytes: usize,
    #[serde(default = "default_insecure_transport_warn_interval_secs")]
    pub insecure_transport_warn_interval_secs: u64,
}

/// 从 TOML 文本中解析并校验出 `ServerConfig`。
pub fn parse_server_config(input: &str) -> Result<ServerConfig, ConfigError> {
    let raw: RawServerConfigFile =
        toml::from_str(input).map_err(|error| ConfigError::new(error.to_string()))?;
    raw.validate()
}

/// 从 TOML 文本中解析并校验出 `AgentConfig`。
pub fn parse_agent_config(input: &str) -> Result<AgentConfig, ConfigError> {
    let raw: RawAgentConfigFile =
        toml::from_str(input).map_err(|error| ConfigError::new(error.to_string()))?;
    raw.validate()
}
