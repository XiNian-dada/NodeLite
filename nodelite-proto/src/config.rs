//! 配置文件解析:Agent 与 Server 启动时读取的 TOML 配置。
//!
//! 设计要点:
//! 1. 暴露的 `ServerConfig` / [`AgentConfig`] 是经过校验的"干净"结构。
//! 2. 原始 TOML 反序列化、默认值与内部校验 helper 分拆到子模块中,保持公开 API 稳定。
//! 3. 所有默认值通过常量 `DEFAULT_*` 暴露,供本模块与外部组件共享。

#[macro_use]
mod macros;

#[cfg(feature = "server-config")]
mod alerts;
mod defaults;
mod edit;
mod helpers;
mod raw;
#[cfg(feature = "server-config")]
mod server;
#[cfg(test)]
mod tests;

use serde::{Deserialize, Serialize};

use self::defaults::{
    default_agent_auth_timeout_secs, default_agent_ignored_filesystems,
    default_agent_inbound_timeout_secs, default_agent_send_timeout_secs,
    default_connect_timeout_secs, default_insecure_transport_warn_interval_secs,
    default_max_incoming_message_bytes,
};
use self::raw::RawAgentConfigFile;

#[cfg(feature = "server-config")]
pub use self::server::{
    AgentLogsConfig, AuditConfig, GeoIpConfig, GeoIpEdition, GeoIpProvider, MetricsConfig,
    ReadonlyAuthConfig, ServerConfig, WsConfig, parse_server_config,
};

#[cfg(feature = "server-config")]
pub use self::alerts::{
    AlertChannel, AlertComparator, AlertMetric, AlertRuleConfig, AlertScopeMode, AlertSeverity,
    AlertSmtpConfig, AlertSmtpTransport, AlertWebhookConfig, AlertingConfig,
    DEFAULT_ALERT_CPU_WINDOW_MINUTES, DEFAULT_ALERT_INSPECTION_CPU_WARN_PERCENT,
    DEFAULT_ALERT_INSPECTION_LATENCY_WARN_MS, DEFAULT_ALERT_INSPECTION_LOCAL_TIME,
    DEFAULT_ALERT_INSPECTION_LOOKBACK_HOURS, DEFAULT_ALERT_INSPECTION_MEMORY_WARN_PERCENT,
    DEFAULT_ALERT_INSPECTION_OFFLINE_GRACE_MINUTES, DEFAULT_ALERT_MEMORY_WINDOW_MINUTES,
    DEFAULT_ALERT_OFFLINE_THRESHOLD_MINUTES, DEFAULT_ALERT_RTT_WINDOW_MINUTES,
    DEFAULT_ALERT_RULE_COOLDOWN_MINUTES, DEFAULT_ALERT_RULE_WINDOW_MINUTES, InspectionConfig,
};
pub use self::edit::upsert_toml_item_preserving_decor;
#[cfg(feature = "server-config")]
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
/// 同时执行的历史 SQLite 只读查询默认上限。
pub const DEFAULT_HISTORY_QUERY_CONCURRENCY: usize = 4;
/// 历史只读连接的 SQLite 私有 page cache 默认大小(KiB)。
pub const DEFAULT_HISTORY_READ_CACHE_KIB: u64 = 512;
/// 历史写入器单次事务默认最多写入的记录数。
pub const DEFAULT_HISTORY_WRITER_BATCH_MAX: usize = 128;
/// 历史写入器默认最大攒批时间(毫秒)。
pub const DEFAULT_HISTORY_WRITER_FLUSH_INTERVAL_MS: u64 = 100;
/// 历史查询至少保留一个并发槽位。
pub const MIN_HISTORY_QUERY_CONCURRENCY: usize = 1;
/// 防止大量并发 SQLite page cache 抬高匿名内存峰值。
pub const MAX_HISTORY_QUERY_CONCURRENCY: usize = 8;
/// 过小的 page cache 会让覆盖索引查询反复读取相同页面。
pub const MIN_HISTORY_READ_CACHE_KIB: u64 = 64;
/// 单连接 page cache 的运维安全上限(KiB)。
pub const MAX_HISTORY_READ_CACHE_KIB: u64 = 1024;
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
/// 节点身份中单个展示文本字段允许的最大 UTF-8 字节数。
pub const MAX_NODE_IDENTITY_TEXT_BYTES: usize = 256;
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
/// Argon2 token 验证的默认最大并发数。
pub const DEFAULT_TOKEN_VERIFY_MAX_PARALLELISM: usize = 4;
/// Argon2 token 验证并发配置的最小安全值。
pub const MIN_TOKEN_VERIFY_MAX_PARALLELISM: usize = 1;
/// Argon2 token 验证并发配置的最大安全值。
pub const MAX_TOKEN_VERIFY_MAX_PARALLELISM: usize = 8;
/// 审计日志默认保留天数。
pub const DEFAULT_AUDIT_RETENTION_DAYS: u64 = 90;
/// 审计写入器单次事务默认最多写入的记录数。
pub const DEFAULT_AUDIT_WRITER_BATCH_MAX: usize = 128;
/// 审计写入器默认最大攒批时间(毫秒)。
pub const DEFAULT_AUDIT_WRITER_FLUSH_INTERVAL_MS: u64 = 100;
/// 历史与审计写入器单次事务允许的最大记录数。
pub const MAX_WRITER_BATCH_SIZE: usize = 4096;
/// 写入器 flush 间隔下限，避免极短定时器形成忙循环。
pub const MIN_WRITER_FLUSH_INTERVAL_MS: u64 = 10;
/// GeoIP 数据库默认更新间隔(天)。
pub const DEFAULT_GEOIP_UPDATE_INTERVAL_DAYS: u64 = 30;
/// Agent 连接超时(秒)。
pub const DEFAULT_CONNECT_TIMEOUT_SECS: u64 = 20;
/// Upgrade 成功后等待认证响应的默认期限。
pub const DEFAULT_AGENT_AUTH_TIMEOUT_SECS: u64 = 20;
/// 单次 WebSocket 写入与 flush 的默认期限。
pub const DEFAULT_AGENT_SEND_TIMEOUT_SECS: u64 = 20;
/// 默认容忍九个服务端心跳间隔的入站静默。
pub const DEFAULT_AGENT_INBOUND_TIMEOUT_SECS: u64 = 90;
/// Agent 最大接收消息字节数。
pub const DEFAULT_MAX_INCOMING_MESSAGE_BYTES: usize = 64 * 1024;

/// Pseudo filesystems are excluded at collection time to bound wire and Prometheus cardinality.
pub const DEFAULT_AGENT_IGNORED_FILESYSTEMS: &[&str] = &[
    "autofs",
    "bpf",
    "cgroup",
    "cgroup2",
    "configfs",
    "debugfs",
    "devfs",
    "devpts",
    "devtmpfs",
    "fdesc",
    "fusectl",
    "mqueue",
    "overlay",
    "proc",
    "procfs",
    "pstore",
    "ramfs",
    "securityfs",
    "squashfs",
    "sysfs",
    "tmpfs",
    "tracefs",
    "volfs",
];

/// 配置加载或校验过程中产生的错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigError {
    message: String,
}

impl ConfigError {
    /// 用可读错误消息创建配置错误。
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

impl From<crate::validation::ValidationError> for ConfigError {
    fn from(error: crate::validation::ValidationError) -> Self {
        Self::new(error.to_string())
    }
}

/// Agent 启动需要的全部配置。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentConfig {
    /// Agent 在 server registry 中的稳定节点 ID。
    pub node_id: String,
    /// UI 中展示的节点名称。
    pub node_label: String,
    /// Server WebSocket 或基础连接地址。
    pub server: String,
    /// Agent 连接 server 使用的认证 token。
    pub token: String,
    /// Agent 上报指标的间隔秒数。
    pub report_interval_secs: u64,
    /// Replaces the default pseudo-filesystem filter; an empty list opts into all filesystem types.
    #[serde(default = "default_agent_ignored_filesystems")]
    pub ignored_filesystems: Vec<String>,
    /// 可选 hostname 覆盖值,为空时使用本机 hostname。
    pub hostname_override: Option<String>,
    /// 部署方自定义标签,用于筛选和告警作用域。
    pub tags: Vec<String>,
    #[serde(default = "default_connect_timeout_secs")]
    /// Agent 建立连接的超时秒数。
    pub connect_timeout_secs: u64,
    #[serde(default = "default_agent_auth_timeout_secs")]
    /// Upgrade 后等待认证成功的期限，Ping 不会延长期限。
    pub auth_timeout_secs: u64,
    #[serde(default = "default_agent_send_timeout_secs")]
    /// 单次发送（包括 flush）的超时秒数。
    pub send_timeout_secs: u64,
    #[serde(default = "default_agent_inbound_timeout_secs")]
    /// 认证后允许的最长入站静默，需大于服务端的心跳间隔。
    pub inbound_timeout_secs: u64,
    #[serde(default = "default_max_incoming_message_bytes")]
    /// Agent 接受的单条 server 消息最大字节数。
    pub max_incoming_message_bytes: usize,
    #[serde(default = "default_insecure_transport_warn_interval_secs")]
    /// 明文传输警告的最小重复提示间隔秒数。
    pub insecure_transport_warn_interval_secs: u64,
}

/// 从 TOML 文本中解析并校验出 `AgentConfig`。
pub fn parse_agent_config(input: &str) -> Result<AgentConfig, ConfigError> {
    let raw: RawAgentConfigFile =
        toml::from_str(input).map_err(|error| ConfigError::new(error.to_string()))?;
    raw.validate()
}
