//! Validated server settings, kept out of Agent-only dependency graphs.

use std::net::SocketAddr;
use std::path::PathBuf;

use ipnet::IpNet;
use serde::{Deserialize, Serialize};

use super::defaults::{
    default_audit_writer_batch_max, default_audit_writer_flush_interval_ms,
    default_hello_timeout_secs, default_history_query_concurrency, default_history_read_cache_kib,
    default_history_writer_batch_max, default_history_writer_flush_interval_ms,
    default_insecure_transport_warn_interval_secs, default_max_outstanding_pings,
    default_max_sanitized_disks, default_max_sanitized_string_bytes,
    default_metric_anomaly_session_limit, default_metrics_export_node_disk_metrics,
    default_metrics_export_node_resource_metrics, default_sqlite_busy_timeout_secs,
    default_token_verify_max_parallelism,
};
use super::raw::RawServerConfigFile;
use super::{AlertingConfig, ConfigError};

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
    #[serde(default = "default_history_query_concurrency")]
    /// 同时执行的历史 SQLite 只读查询上限。
    pub history_query_concurrency: usize,
    #[serde(default = "default_history_read_cache_kib")]
    /// 每个历史只读连接的 SQLite 私有 page cache 大小(KiB)。
    pub history_read_cache_kib: u64,
    #[serde(default = "default_history_writer_batch_max")]
    /// 历史写入器单次事务最多写入的记录数。
    pub history_writer_batch_max: usize,
    #[serde(default = "default_history_writer_flush_interval_ms")]
    /// 历史写入器最大攒批时间(毫秒)。
    pub history_writer_flush_interval_ms: u64,
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
    #[serde(default = "default_token_verify_max_parallelism")]
    /// 同时执行的 Argon2 token 验证任务上限。
    pub token_verify_max_parallelism: usize,
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
    #[serde(default = "default_audit_writer_batch_max")]
    /// 审计写入器单次事务最多写入的记录数。
    pub writer_batch_max: usize,
    #[serde(default = "default_audit_writer_flush_interval_ms")]
    /// 审计写入器最大攒批时间(毫秒)。
    pub writer_flush_interval_ms: u64,
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

/// 从 TOML 文本中解析并校验出 `ServerConfig`。
pub fn parse_server_config(input: &str) -> Result<ServerConfig, ConfigError> {
    let raw: RawServerConfigFile =
        toml::from_str(input).map_err(|error| ConfigError::new(error.to_string()))?;
    raw.validate()
}
