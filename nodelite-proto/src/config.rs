//! 配置文件解析:Agent 与 Server 启动时读取的 TOML 配置。
//!
//! 设计要点:
//! 1. 暴露的 [`ServerConfig`]/[`AgentConfig`] 是经过校验的"干净"结构。
//! 2. 原始 TOML 反序列化、默认值与内部校验 helper 分拆到子模块中,保持公开 API 稳定。
//! 3. 所有默认值通过常量 `DEFAULT_*` 暴露,供本模块与外部组件共享。

mod alerts;
mod defaults;
mod edit;
mod helpers;
mod raw;
#[cfg(test)]
mod tests;

use std::net::SocketAddr;
use std::path::PathBuf;

use ipnet::IpNet;
use serde::{Deserialize, Serialize};

use self::defaults::{
    default_connect_timeout_secs, default_hello_timeout_secs,
    default_insecure_transport_warn_interval_secs, default_max_incoming_message_bytes,
    default_max_outstanding_pings, default_max_sanitized_disks, default_max_sanitized_string_bytes,
    default_metric_anomaly_session_limit, default_metrics_export_node_disk_metrics,
    default_metrics_export_node_resource_metrics, default_sqlite_busy_timeout_secs,
};
use self::raw::{RawAgentConfigFile, RawServerConfigFile};

pub use self::alerts::{
    AlertChannel, AlertComparator, AlertMetric, AlertRuleConfig, AlertScopeMode, AlertSeverity,
    AlertSmtpConfig, AlertSmtpTransport, AlertWebhookConfig, AlertingConfig,
    DEFAULT_ALERT_INSPECTION_CPU_WARN_PERCENT, DEFAULT_ALERT_INSPECTION_LATENCY_WARN_MS,
    DEFAULT_ALERT_INSPECTION_LOCAL_TIME, DEFAULT_ALERT_INSPECTION_LOOKBACK_HOURS,
    DEFAULT_ALERT_INSPECTION_MEMORY_WARN_PERCENT, DEFAULT_ALERT_INSPECTION_OFFLINE_GRACE_MINUTES,
    DEFAULT_ALERT_RULE_COOLDOWN_MINUTES, DEFAULT_ALERT_RULE_WINDOW_MINUTES, InspectionConfig,
};
pub use self::edit::upsert_toml_item_preserving_decor;
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
/// GeoIP 数据库默认更新间隔(天)。
pub const DEFAULT_GEOIP_UPDATE_INTERVAL_DAYS: u64 = 30;
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

/// Server 启动需要的全部配置。
///
/// #98: **故意不派生 `Serialize`**。`ServerConfig` 持有 `readonly_auth.password`
/// 与 `readonly_auth.totp_secret`,如果允许把整个结构直接序列化(`Json(config)`
/// / `serde_json::to_string(&config)`),任何一处疏忽就会让明文凭证泄露到响应、
/// 日志或调试输出。需要对外暴露字段时,请在 handler 内手工构造一个不带敏感字段
/// 的视图类型(参考 `handlers/settings/mod.rs::SettingsResponse`)。
///
/// ```compile_fail
/// fn assert_serializable<T: serde::Serialize>() {}
/// assert_serializable::<nodelite_proto::ServerConfig>();
/// ```
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct ServerConfig {
    /// Server 监听地址和端口。
    pub listen: SocketAddr,
    /// 对外访问 Server 的基础 URL,用于生成安装脚本和提示信息。
    pub public_base_url: String,
    /// 是否允许 `public_base_url` 使用明文 HTTP。
    pub insecure_allow_http: bool,
    /// 可信反向代理网段,用于解析真实客户端 IP。
    pub trusted_proxies: Vec<IpNet>,
    /// 可选的只读 Web UI 认证配置。
    pub readonly_auth: Option<ReadonlyAuthConfig>,
    /// WebSocket 准入和认证失败限流配置。
    pub ws: WsConfig,
    /// Prometheus 指标导出配置。
    pub metrics: MetricsConfig,
    /// 审计日志配置。
    pub audit: AuditConfig,
    /// GeoIP 数据源与更新配置。
    pub geoip: GeoIpConfig,
    /// 告警规则、巡检和通知渠道配置。
    pub alerting: AlertingConfig,
    /// 节点注册表持久化文件路径。
    pub node_registry_path: PathBuf,
    /// 历史指标 SQLite 数据库路径。
    pub history_db_path: PathBuf,
    /// 最新快照持久化文件路径。
    pub snapshot_path: PathBuf,
    /// 超过该秒数未收到上报后,节点视为离线。
    pub stale_after_secs: u64,
    /// Server 发送 WebSocket ping 的间隔秒数。
    pub ping_interval_secs: u64,
    /// Server 接受的单条 WebSocket 消息最大字节数。
    pub max_message_bytes: usize,
    /// 前端轮询或刷新 fallback 的默认间隔秒数。
    pub refresh_interval_secs: u64,
    /// Agent 默认过滤的文件系统类型列表。
    pub ignored_filesystems: Vec<String>,
    /// Agent release 下载基础 URL,为空时使用项目默认发布地址。
    pub agent_release_base_url: Option<String>,
    /// x86_64 Linux Agent release 的 SHA-256 校验值。
    pub agent_release_sha256_x86_64: Option<String>,
    /// aarch64 Linux Agent release 的 SHA-256 校验值。
    pub agent_release_sha256_aarch64: Option<String>,
    #[serde(default = "default_hello_timeout_secs")]
    /// WebSocket hello 握手阶段的超时秒数。
    pub hello_timeout_secs: u64,
    #[serde(default = "default_max_outstanding_pings")]
    /// 单连接允许的最大未响应 ping 数。
    pub max_outstanding_pings: usize,
    #[serde(default = "default_insecure_transport_warn_interval_secs")]
    /// 明文传输安全告警的最小重复提示间隔秒数。
    pub insecure_transport_warn_interval_secs: u64,
    #[serde(default = "default_max_sanitized_disks")]
    /// 单个快照保留的最大磁盘条目数。
    pub max_sanitized_disks: usize,
    #[serde(default = "default_max_sanitized_string_bytes")]
    /// 快照字符串字段清洗后的最大 UTF-8 字节数。
    pub max_sanitized_string_bytes: usize,
    #[serde(default = "default_metric_anomaly_session_limit")]
    /// 同一会话允许记录的指标异常次数上限。
    pub metric_anomaly_session_limit: usize,
    #[serde(default = "default_sqlite_busy_timeout_secs")]
    /// SQLite busy timeout 秒数。
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
    /// 只读 Web UI 登录用户名。
    pub username: String,
    /// 只读 Web UI 登录密码明文,仅存在于本地配置中。
    pub password: String,
    #[serde(default)]
    /// 是否启用 TOTP 二次验证。
    pub enable_2fa: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// TOTP secret,启用 2FA 时由 server 读取和校验。
    pub totp_secret: Option<String>,
}

/// WebSocket 准入控制参数,用于限流与抗暴力破解。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WsConfig {
    /// 全局允许的最大 WebSocket 连接数。
    pub max_total_connections: usize,
    /// 单个客户端 IP 允许的最大 WebSocket 连接数。
    pub max_connections_per_ip: usize,
    /// 认证失败计数窗口秒数。
    pub auth_fail_window_secs: u64,
    /// 计数窗口内触发封禁的最大认证失败次数。
    pub auth_fail_max_attempts: usize,
    /// 认证失败触发后的封禁秒数。
    pub auth_block_secs: u64,
}

/// Prometheus 导出粒度控制。默认保持轻量 summary,细节点资源指标需显式打开。
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct MetricsConfig {
    #[serde(default = "default_metrics_export_node_resource_metrics")]
    /// 是否导出每节点 CPU、内存、网络等资源指标。
    pub export_node_resource_metrics: bool,
    #[serde(default = "default_metrics_export_node_disk_metrics")]
    /// 是否按节点和挂载点导出磁盘指标。
    pub export_node_disk_metrics: bool,
}

/// 审计日志存储与记录策略。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AuditConfig {
    /// 是否启用审计日志。
    pub enabled: bool,
    /// 审计日志 SQLite 数据库路径。
    pub db_path: PathBuf,
    /// 审计记录保留天数。
    pub retention_days: u64,
    /// 是否记录成功认证事件。
    pub log_successful_auth: bool,
    /// 是否记录失败认证事件。
    pub log_failed_auth: bool,
    /// 是否记录 token 签发和刷新事件。
    pub log_token_events: bool,
    /// 是否记录限流或封禁事件。
    pub log_rate_limit: bool,
}

/// IP 地理位置数据库配置。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GeoIpConfig {
    /// 是否启用 GeoIP 推断。
    pub enabled: bool,
    /// GeoIP 数据来源。
    pub provider: GeoIpProvider,
    /// GeoIP 数据库粒度。
    pub edition: GeoIpEdition,
    /// 本地 GeoIP 数据库路径。
    pub database_path: PathBuf,
    /// 是否允许 server 自动更新 GeoIP 数据库。
    pub auto_update: bool,
    /// 自动更新间隔天数。
    pub update_interval_days: u64,
}

/// GeoIP 数据来源。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum GeoIpProvider {
    /// DB-IP Lite 数据库。
    Dbip,
    /// ipwho.is HTTP API。
    Ipwhois,
    /// 用户提供的自定义数据库或后续扩展源。
    Custom,
}

/// DB-IP Lite 数据库粒度。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum GeoIpEdition {
    /// 国家级 DB-IP Lite 数据库。
    CountryLite,
    /// 城市级 DB-IP Lite 数据库。
    CityLite,
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
    /// 可选 hostname 覆盖值,为空时使用本机 hostname。
    pub hostname_override: Option<String>,
    /// 部署方自定义标签,用于筛选和告警作用域。
    pub tags: Vec<String>,
    #[serde(default = "default_connect_timeout_secs")]
    /// Agent 建立连接的超时秒数。
    pub connect_timeout_secs: u64,
    #[serde(default = "default_max_incoming_message_bytes")]
    /// Agent 接受的单条 server 消息最大字节数。
    pub max_incoming_message_bytes: usize,
    #[serde(default = "default_insecure_transport_warn_interval_secs")]
    /// 明文传输警告的最小重复提示间隔秒数。
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
