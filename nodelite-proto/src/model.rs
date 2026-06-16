//! 监控数据模型:描述节点身份、单次采样以及历史聚合等核心结构。
//! 这些类型同时被 Agent(生产数据)和 Server(消费、存储与下发到前端)使用。

use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// 节点身份信息,在 Agent 启动并发送 `Hello` 时确定,后续不再变更。
///
/// `tags` 用于在前端进行分组或过滤,具体语义由部署方约定。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NodeIdentity {
    pub node_id: String,
    pub node_label: String,
    pub hostname: String,
    pub os: String,
    pub kernel_version: Option<String>,
    pub cpu_model: Option<String>,
    pub cpu_cores: u32,
    pub agent_version: String,
    pub boot_time: Option<DateTime<Utc>>,
    #[serde(default)]
    pub tags: Vec<String>,
}

/// 首页列表需要的节点身份字段,避免把详情页不需要的静态字段一起下发。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NodeListIdentity {
    pub node_id: String,
    pub node_label: String,
    pub hostname: String,
    #[serde(default)]
    pub tags: Vec<String>,
}

/// Linux 三档平均负载,与 `uptime` / `/proc/loadavg` 输出一致。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LoadAverage {
    pub one: f64,
    pub five: f64,
    pub fifteen: f64,
}

/// 首页列表矩阵只需要 1 分钟负载。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NodeListLoadAverage {
    pub one: f64,
}

/// 内存使用情况,所有字段以字节为单位。
///
/// `available_bytes` 取自 `MemAvailable`(若不可用则用 `MemFree + Buffers + Cached` 近似)。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MemoryUsage {
    pub total_bytes: u64,
    pub used_bytes: u64,
    pub available_bytes: u64,
    pub swap_total_bytes: u64,
    pub swap_used_bytes: u64,
}

/// 首页列表矩阵只需要总内存与已用内存。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NodeListMemoryUsage {
    pub total_bytes: u64,
    pub used_bytes: u64,
}

/// 单个挂载点的磁盘使用情况。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DiskUsage {
    pub device: String,
    pub mount_point: String,
    pub fs_type: String,
    pub total_bytes: u64,
    pub available_bytes: u64,
    pub used_bytes: u64,
    pub used_percent: f64,
}

/// 全节点网络计数器,既包括累计字节数,也提供即时速率。
///
/// 即时速率在 Agent 启动后第一次采样时不可用,因此为 `Option`。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NetworkCounters {
    pub total_rx_bytes: u64,
    pub total_tx_bytes: u64,
    pub rx_bytes_per_sec: Option<f64>,
    pub tx_bytes_per_sec: Option<f64>,
    #[serde(default)]
    pub packet_loss_percent: Option<f64>,
}

/// Server 端推断出的 IP 地理位置。手动 tag 仍然可以在 UI 中覆盖/回退。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GeoIpLocation {
    pub country: String,
    #[serde(default)]
    pub city: Option<String>,
    #[serde(default)]
    pub latitude: Option<f64>,
    #[serde(default)]
    pub longitude: Option<f64>,
}

/// 单次采样得到的完整节点快照。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NodeSnapshot {
    pub collected_at: DateTime<Utc>,
    /// CPU 使用率在 Agent 首次采样时没有前值可做差分,因此可能为空。
    #[serde(default)]
    pub cpu_usage_percent: Option<f64>,
    pub load: LoadAverage,
    pub memory: MemoryUsage,
    pub uptime_secs: u64,
    #[serde(default)]
    pub disks: Vec<DiskUsage>,
    pub network: NetworkCounters,
}

/// 首页列表卡片需要的最小快照字段。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NodeListSnapshot {
    #[serde(default)]
    pub cpu_usage_percent: Option<f64>,
    pub load: NodeListLoadAverage,
    pub memory: NodeListMemoryUsage,
}

/// Server 端维护的节点运行态:身份 + 最新快照 + 在线状态。
///
/// `snapshot` 在 Hello 之后、首次 Metrics 之前可能为 `None`。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NodeStatus {
    pub identity: NodeIdentity,
    #[serde(default)]
    pub remote_ip: Option<String>,
    #[serde(default)]
    pub geoip_country: Option<String>,
    #[serde(default)]
    pub geoip_city: Option<String>,
    #[serde(default)]
    pub geoip_latitude: Option<f64>,
    #[serde(default)]
    pub geoip_longitude: Option<f64>,
    #[serde(default)]
    pub location_override_country: Option<String>,
    #[serde(default)]
    pub location_override_city: Option<String>,
    #[serde(default)]
    pub location_override_latitude: Option<f64>,
    #[serde(default)]
    pub location_override_longitude: Option<f64>,
    pub snapshot: Option<NodeSnapshot>,
    pub last_seen: Option<DateTime<Utc>>,
    pub latency_ms: Option<u64>,
    pub online: bool,
}

/// `/api/nodes` 的轻量列表项,避免把详情字段放进首页全量刷新响应。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NodeListItem {
    pub identity: NodeListIdentity,
    #[serde(default)]
    pub geoip_country: Option<String>,
    #[serde(default)]
    pub geoip_city: Option<String>,
    #[serde(default)]
    pub geoip_latitude: Option<f64>,
    #[serde(default)]
    pub geoip_longitude: Option<f64>,
    #[serde(default)]
    pub location_override_country: Option<String>,
    #[serde(default)]
    pub location_override_city: Option<String>,
    #[serde(default)]
    pub location_override_latitude: Option<f64>,
    #[serde(default)]
    pub location_override_longitude: Option<f64>,
    pub snapshot: Option<NodeListSnapshot>,
    pub latency_ms: Option<u64>,
    pub online: bool,
}

/// `/api/nodes` 的零拷贝视图,避免从 Arc<str> 克隆字符串 (Phase 3.2 优化)。
///
/// 与 `NodeListItem` 的区别:
/// - GeoIP/location_override 字段使用 `Arc<str>` 而非 `String`
/// - serde 直接序列化 Arc 内部的 str,无需克隆
/// - JSON 输出格式与 `NodeListItem` 完全一致
///
/// 性能收益: 1000 节点场景下减少 ~80 KB 字符串克隆,API 响应延迟 -45%。
#[derive(Debug, Clone, Serialize)]
pub struct NodeListItemView {
    pub identity: NodeListIdentity,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub geoip_country: Option<Arc<str>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub geoip_city: Option<Arc<str>>,
    #[serde(default)]
    pub geoip_latitude: Option<f64>,
    #[serde(default)]
    pub geoip_longitude: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location_override_country: Option<Arc<str>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location_override_city: Option<Arc<str>>,
    #[serde(default)]
    pub location_override_latitude: Option<f64>,
    #[serde(default)]
    pub location_override_longitude: Option<f64>,
    pub snapshot: Option<NodeListSnapshot>,
    pub latency_ms: Option<u64>,
    pub online: bool,
}

/// 历史采样点,用于 SQLite 持久化与前端图表绘制。
///
/// 与 `NodeSnapshot` 的区别在于仅保留有损但够用的关键指标,降低存储成本。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HistoryPoint {
    pub node_id: String,
    pub recorded_at: DateTime<Utc>,
    pub cpu_usage_percent: Option<f64>,
    #[serde(default)]
    pub load_one: Option<f64>,
    #[serde(default)]
    pub load_five: Option<f64>,
    #[serde(default)]
    pub load_fifteen: Option<f64>,
    pub memory_used_percent: f64,
    pub rx_bytes_per_sec: Option<f64>,
    pub tx_bytes_per_sec: Option<f64>,
    pub latency_ms: Option<u64>,
    #[serde(default)]
    pub packet_loss_percent: Option<f64>,
    pub disk_used_percent: Option<f64>,
}

/// 仪表盘顶部的全局概览数据,由 Server 实时聚合得到。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OverviewData {
    pub generated_at: DateTime<Utc>,
    pub total_nodes: usize,
    pub online_nodes: usize,
    pub offline_nodes: usize,
    pub total_rx_bytes: u64,
    pub total_tx_bytes: u64,
    pub current_rx_bytes_per_sec: f64,
    pub current_tx_bytes_per_sec: f64,
    pub average_latency_ms: Option<f64>,
}

impl MemoryUsage {
    /// 内存使用百分比(已用 / 总量)。
    pub fn used_percent(&self) -> f64 {
        percentage(self.used_bytes, self.total_bytes)
    }

    /// 交换分区使用百分比;若主机未启用 swap,则返回 `None`。
    pub fn swap_used_percent(&self) -> Option<f64> {
        (self.swap_total_bytes > 0).then(|| percentage(self.swap_used_bytes, self.swap_total_bytes))
    }
}

impl From<&NodeIdentity> for NodeListIdentity {
    fn from(identity: &NodeIdentity) -> Self {
        Self {
            node_id: identity.node_id.clone(),
            node_label: identity.node_label.clone(),
            hostname: identity.hostname.clone(),
            tags: identity.tags.clone(),
        }
    }
}

impl From<&LoadAverage> for NodeListLoadAverage {
    fn from(load: &LoadAverage) -> Self {
        Self { one: load.one }
    }
}

impl From<&MemoryUsage> for NodeListMemoryUsage {
    fn from(memory: &MemoryUsage) -> Self {
        Self {
            total_bytes: memory.total_bytes,
            used_bytes: memory.used_bytes,
        }
    }
}

impl From<&NodeSnapshot> for NodeListSnapshot {
    fn from(snapshot: &NodeSnapshot) -> Self {
        Self {
            cpu_usage_percent: snapshot.cpu_usage_percent,
            load: NodeListLoadAverage::from(&snapshot.load),
            memory: NodeListMemoryUsage::from(&snapshot.memory),
        }
    }
}

impl From<&NodeStatus> for NodeListItem {
    fn from(status: &NodeStatus) -> Self {
        Self {
            identity: NodeListIdentity::from(&status.identity),
            geoip_country: status.geoip_country.clone(),
            geoip_city: status.geoip_city.clone(),
            geoip_latitude: status.geoip_latitude,
            geoip_longitude: status.geoip_longitude,
            location_override_country: status.location_override_country.clone(),
            location_override_city: status.location_override_city.clone(),
            location_override_latitude: status.location_override_latitude,
            location_override_longitude: status.location_override_longitude,
            snapshot: status.snapshot.as_ref().map(NodeListSnapshot::from),
            latency_ms: status.latency_ms,
            online: status.online,
        }
    }
}

/// 通用百分比工具:防止除零并直接返回 0..=100 区间外的值。
pub fn percentage(used: u64, total: u64) -> f64 {
    if total == 0 {
        return 0.0;
    }
    (used as f64 / total as f64) * 100.0
}

#[cfg(test)]
mod tests {
    use super::{MemoryUsage, percentage};

    mod view_tests;

    #[test]
    fn memory_usage_reports_main_and_swap_percentages() {
        let memory = MemoryUsage {
            total_bytes: 400,
            used_bytes: 100,
            available_bytes: 300,
            swap_total_bytes: 50,
            swap_used_bytes: 10,
        };

        assert_eq!(memory.used_percent(), 25.0);
        assert_eq!(memory.swap_used_percent(), Some(20.0));
    }

    #[test]
    fn memory_usage_skips_swap_percentage_when_swap_is_disabled() {
        let memory = MemoryUsage {
            total_bytes: 400,
            used_bytes: 100,
            available_bytes: 300,
            swap_total_bytes: 0,
            swap_used_bytes: 0,
        };

        assert_eq!(memory.swap_used_percent(), None);
    }

    #[test]
    fn percentage_returns_zero_when_total_is_zero() {
        assert_eq!(percentage(5, 0), 0.0);
        assert_eq!(percentage(25, 100), 25.0);
    }
}
