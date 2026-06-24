use serde::{Deserialize, Serialize};

/// 默认告警规则评估窗口(分钟)。
pub const DEFAULT_ALERT_RULE_WINDOW_MINUTES: u64 = 5;
/// 默认告警冷却时间(分钟)。
pub const DEFAULT_ALERT_RULE_COOLDOWN_MINUTES: u64 = 30;
/// 默认每日巡检回看窗口(小时)。
pub const DEFAULT_ALERT_INSPECTION_LOOKBACK_HOURS: u64 = 24;
/// 默认每日巡检本地时间。
pub const DEFAULT_ALERT_INSPECTION_LOCAL_TIME: &str = "09:00";
/// 默认离线巡检宽限时间(分钟)。
pub const DEFAULT_ALERT_INSPECTION_OFFLINE_GRACE_MINUTES: u64 = 10;
/// 默认延迟告警阈值(毫秒)。
pub const DEFAULT_ALERT_INSPECTION_LATENCY_WARN_MS: u64 = 250;
/// 默认 CPU 使用率告警阈值(百分比)。
pub const DEFAULT_ALERT_INSPECTION_CPU_WARN_PERCENT: u64 = 85;
/// 默认内存使用率告警阈值(百分比)。
pub const DEFAULT_ALERT_INSPECTION_MEMORY_WARN_PERCENT: u64 = 90;

/// 告警投递渠道。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AlertChannel {
    /// SMTP 邮件渠道。
    Smtp,
    /// HTTP webhook 渠道。
    Webhook,
}

/// SMTP 连接安全模式。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AlertSmtpTransport {
    /// 先明文连接再通过 STARTTLS 升级。
    StartTls,
    /// 从连接开始使用 TLS。
    Tls,
    /// 明文 SMTP,仅适用于受信内网或测试环境。
    Plain,
}

/// 告警规则可观察的指标。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AlertMetric {
    /// CPU 使用率百分比。
    CpuUsagePercent,
    /// 内存使用率百分比。
    MemoryUsagePercent,
    /// 磁盘使用率百分比。
    DiskUsagePercent,
    /// WebSocket 心跳测得的延迟毫秒数。
    LatencyMs,
    /// 节点离线持续分钟数。
    OfflineMinutes,
}

/// 告警阈值比较方式。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AlertComparator {
    /// 指标值大于阈值时触发。
    Gt,
    /// 指标值小于阈值时触发。
    Lt,
}

/// 告警严重级别。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AlertSeverity {
    /// 需要关注但未达到严重故障级别。
    Warning,
    /// 需要立即处理的严重告警。
    Critical,
}

/// 告警规则的作用域类型。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AlertScopeMode {
    /// 应用于所有节点。
    All,
    /// 仅应用于指定节点 ID。
    NodeIds,
    /// 应用于带有指定标签的节点。
    Tags,
}

/// SMTP 告警渠道配置。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AlertSmtpConfig {
    /// 是否启用 SMTP 渠道。
    pub enabled: bool,
    /// SMTP 服务器主机名或 IP。
    pub host: String,
    /// SMTP 服务器端口。
    pub port: u16,
    /// SMTP 登录用户名。
    pub username: String,
    /// SMTP 登录密码或应用专用密码。
    pub password: Option<String>,
    /// 告警邮件发件人地址。
    pub sender: String,
    /// 告警邮件收件人列表。
    pub recipients: Vec<String>,
    /// SMTP 传输安全模式。
    pub transport: AlertSmtpTransport,
    /// 告警恢复时是否发送 resolved 通知。
    pub send_resolved: bool,
}

impl Default for AlertSmtpConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            host: String::new(),
            port: 587,
            username: String::new(),
            password: None,
            sender: String::new(),
            recipients: Vec::new(),
            transport: AlertSmtpTransport::StartTls,
            send_resolved: true,
        }
    }
}

/// Webhook 告警渠道配置。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AlertWebhookConfig {
    /// 是否启用 webhook 渠道。
    pub enabled: bool,
    /// 接收告警的 webhook URL。
    pub url: String,
    /// 可选共享 secret,用于服务端签名或下游校验。
    pub secret: Option<String>,
    /// 告警恢复时是否发送 resolved 通知。
    pub send_resolved: bool,
}

impl Default for AlertWebhookConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            url: String::new(),
            secret: None,
            send_resolved: true,
        }
    }
}

/// 单条阈值告警规则。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AlertRuleConfig {
    /// 规则稳定 ID。
    pub id: String,
    /// UI 中展示的规则名称。
    pub name: String,
    /// 是否启用该规则。
    pub enabled: bool,
    /// 规则观察的指标。
    pub metric: AlertMetric,
    /// 指标和阈值的比较方式。
    pub comparator: AlertComparator,
    /// 告警阈值,单位由 `metric` 决定。
    pub threshold: u64,
    /// 评估窗口分钟数。
    pub window_minutes: u64,
    /// 触发后的严重级别。
    pub severity: AlertSeverity,
    /// 规则作用域模式。
    pub scope_mode: AlertScopeMode,
    /// 当 `scope_mode` 为 `node_ids` 时匹配的节点 ID。
    pub node_ids: Vec<String>,
    /// 当 `scope_mode` 为 `tags` 时匹配的节点标签。
    pub tags: Vec<String>,
    /// 触发后投递到的渠道列表。
    pub delivery: Vec<AlertChannel>,
    /// 同一告警重复通知的冷却分钟数。
    pub cooldown_minutes: u64,
    /// 告警恢复时是否发送 resolved 通知。
    pub send_resolved: bool,
}

/// 每日巡检摘要配置。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct InspectionConfig {
    /// 是否启用每日巡检。
    pub enabled: bool,
    /// 每日巡检触发的本地时间,格式为 `HH:MM`。
    pub local_time: String,
    /// 巡检统计回看窗口(小时)。
    pub lookback_hours: u64,
    /// 巡检摘要投递渠道。
    pub delivery: Vec<AlertChannel>,
    /// 节点离线超过该分钟数后在巡检中标记为异常。
    pub offline_grace_minutes: u64,
    /// 延迟超过该毫秒数后在巡检中标记为异常。
    pub latency_warn_ms: u64,
    /// CPU 使用率超过该百分比后在巡检中标记为异常。
    pub cpu_warn_percent: u64,
    /// 内存使用率超过该百分比后在巡检中标记为异常。
    pub memory_warn_percent: u64,
}

impl Default for InspectionConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            local_time: DEFAULT_ALERT_INSPECTION_LOCAL_TIME.to_string(),
            lookback_hours: DEFAULT_ALERT_INSPECTION_LOOKBACK_HOURS,
            delivery: vec![AlertChannel::Smtp],
            offline_grace_minutes: DEFAULT_ALERT_INSPECTION_OFFLINE_GRACE_MINUTES,
            latency_warn_ms: DEFAULT_ALERT_INSPECTION_LATENCY_WARN_MS,
            cpu_warn_percent: DEFAULT_ALERT_INSPECTION_CPU_WARN_PERCENT,
            memory_warn_percent: DEFAULT_ALERT_INSPECTION_MEMORY_WARN_PERCENT,
        }
    }
}

/// 告警系统的总配置入口。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct AlertingConfig {
    /// 是否启用告警系统。
    pub enabled: bool,
    /// SMTP 渠道配置。
    pub smtp: AlertSmtpConfig,
    /// Webhook 渠道配置。
    pub webhook: AlertWebhookConfig,
    /// 阈值告警规则列表。
    pub rules: Vec<AlertRuleConfig>,
    /// 每日巡检配置。
    pub inspection: InspectionConfig,
}
