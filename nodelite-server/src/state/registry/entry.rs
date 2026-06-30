use std::sync::Arc;

use chrono::{DateTime, Utc};
use nodelite_proto::{
    GeoIpLocation, NodeIdentity, NodeListIdentity, NodeListItem, NodeListItemView,
    NodeListSnapshot, NodeSnapshot, NodeStatus,
};

use super::super::overview::OverviewNode;
use super::super::session_control::SessionControlHandle;
use crate::alerts::AlertStatusView;
use crate::handlers::metrics_routes::PrometheusNode;

/// 单节点的运行态条目。外部响应模型只在 API / snapshot 边界按需组装。
///
/// 字符串池优化: `geoip_country`, `geoip_city`, `location_override_country`, `location_override_city`
/// 使用 Arc<str> 存储,在高重复场景(如 1000 节点同城)大幅降低内存占用。
#[derive(Debug, Clone)]
pub(super) struct NodeEntry {
    pub(super) identity: NodeIdentity,
    pub(super) remote_ip: Option<String>,
    pub(super) geoip_country: Option<Arc<str>>,
    pub(super) geoip_city: Option<Arc<str>>,
    pub(super) geoip_latitude: Option<f64>,
    pub(super) geoip_longitude: Option<f64>,
    pub(super) location_override_country: Option<Arc<str>>,
    pub(super) location_override_city: Option<Arc<str>>,
    pub(super) location_override_latitude: Option<f64>,
    pub(super) location_override_longitude: Option<f64>,
    pub(super) snapshot: Option<NodeSnapshot>,
    pub(super) last_seen: Option<DateTime<Utc>>,
    pub(super) latency_ms: Option<u64>,
    pub(super) online: bool,
    pub(super) active_session_id: Option<u64>,
    pub(super) control: Option<SessionControlHandle>,
}

impl NodeEntry {
    pub(super) fn new(
        session_id: u64,
        identity: NodeIdentity,
        remote_ip: Option<String>,
        geoip: Option<GeoIpLocation>,
        location_override: Option<GeoIpLocation>,
        now: DateTime<Utc>,
        string_pool: &crate::string_pool::StringPool,
    ) -> Self {
        let (geoip_country, geoip_city, geoip_latitude, geoip_longitude) =
            geoip_fields_from_location(geoip.as_ref(), string_pool);
        let (
            location_override_country,
            location_override_city,
            location_override_latitude,
            location_override_longitude,
        ) = geoip_fields_from_location(location_override.as_ref(), string_pool);
        Self {
            identity,
            remote_ip,
            geoip_country,
            geoip_city,
            geoip_latitude,
            geoip_longitude,
            location_override_country,
            location_override_city,
            location_override_latitude,
            location_override_longitude,
            snapshot: None,
            last_seen: Some(now),
            latency_ms: None,
            online: true,
            active_session_id: Some(session_id),
            control: None,
        }
    }

    pub(super) fn from_restored_status(
        mut status: NodeStatus,
        string_pool: &crate::string_pool::StringPool,
    ) -> Self {
        status.online = false;
        Self {
            identity: status.identity,
            remote_ip: status.remote_ip,
            geoip_country: status.geoip_country.as_ref().map(|s| string_pool.intern(s)),
            geoip_city: status.geoip_city.as_ref().map(|s| string_pool.intern(s)),
            geoip_latitude: status.geoip_latitude,
            geoip_longitude: status.geoip_longitude,
            location_override_country: status
                .location_override_country
                .as_ref()
                .map(|s| string_pool.intern(s)),
            location_override_city: status
                .location_override_city
                .as_ref()
                .map(|s| string_pool.intern(s)),
            location_override_latitude: status.location_override_latitude,
            location_override_longitude: status.location_override_longitude,
            snapshot: status.snapshot,
            last_seen: status.last_seen,
            latency_ms: status.latency_ms,
            online: false,
            active_session_id: None,
            control: None,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn register_session(
        &mut self,
        session_id: u64,
        identity: NodeIdentity,
        remote_ip: Option<String>,
        geoip: Option<GeoIpLocation>,
        location_override: Option<GeoIpLocation>,
        now: DateTime<Utc>,
        string_pool: &crate::string_pool::StringPool,
    ) {
        let (geoip_country, geoip_city, geoip_latitude, geoip_longitude) =
            geoip_fields_from_location(geoip.as_ref(), string_pool);
        let (
            location_override_country,
            location_override_city,
            location_override_latitude,
            location_override_longitude,
        ) = geoip_fields_from_location(location_override.as_ref(), string_pool);
        self.identity = identity;
        self.remote_ip = remote_ip;
        self.geoip_country = geoip_country;
        self.geoip_city = geoip_city;
        self.geoip_latitude = geoip_latitude;
        self.geoip_longitude = geoip_longitude;
        self.location_override_country = location_override_country;
        self.location_override_city = location_override_city;
        self.location_override_latitude = location_override_latitude;
        self.location_override_longitude = location_override_longitude;
        self.online = true;
        self.last_seen = Some(now);
        self.latency_ms = None;
        self.active_session_id = Some(session_id);
        self.control = None;
    }

    pub(super) fn to_status(&self) -> NodeStatus {
        NodeStatus {
            identity: self.identity.clone(),
            remote_ip: self.remote_ip.clone(),
            geoip_country: self.geoip_country.as_ref().map(|s| s.to_string()),
            geoip_city: self.geoip_city.as_ref().map(|s| s.to_string()),
            geoip_latitude: self.geoip_latitude,
            geoip_longitude: self.geoip_longitude,
            location_override_country: self
                .location_override_country
                .as_ref()
                .map(|s| s.to_string()),
            location_override_city: self.location_override_city.as_ref().map(|s| s.to_string()),
            location_override_latitude: self.location_override_latitude,
            location_override_longitude: self.location_override_longitude,
            snapshot: self.snapshot.clone(),
            last_seen: self.last_seen,
            latency_ms: self.latency_ms,
            online: self.online,
        }
    }

    pub(super) fn to_summary(&self) -> NodeListItem {
        NodeListItem {
            identity: NodeListIdentity::from(&self.identity),
            geoip_country: self.geoip_country.as_ref().map(|s| s.to_string()),
            geoip_city: self.geoip_city.as_ref().map(|s| s.to_string()),
            geoip_latitude: self.geoip_latitude,
            geoip_longitude: self.geoip_longitude,
            location_override_country: self
                .location_override_country
                .as_ref()
                .map(|s| s.to_string()),
            location_override_city: self.location_override_city.as_ref().map(|s| s.to_string()),
            location_override_latitude: self.location_override_latitude,
            location_override_longitude: self.location_override_longitude,
            snapshot: self.snapshot.as_ref().map(NodeListSnapshot::from),
            latency_ms: self.latency_ms,
            online: self.online,
        }
    }

    /// 零拷贝构建视图 (Phase 3.2 优化)。
    ///
    /// 与 `to_summary()` 的区别:
    /// - 直接克隆 `Arc<str>` (只增加引用计数,不复制字符串)
    /// - 序列化时 serde 直接访问 Arc 内部的 str
    /// - 避免 ~80 KB 字符串克隆 (1000 节点 × 4 字段 × 20 bytes)
    pub(super) fn to_summary_view(&self) -> NodeListItemView {
        NodeListItemView {
            identity: NodeListIdentity::from(&self.identity),
            geoip_country: self.geoip_country.clone(),
            geoip_city: self.geoip_city.clone(),
            geoip_latitude: self.geoip_latitude,
            geoip_longitude: self.geoip_longitude,
            location_override_country: self.location_override_country.clone(),
            location_override_city: self.location_override_city.clone(),
            location_override_latitude: self.location_override_latitude,
            location_override_longitude: self.location_override_longitude,
            snapshot: self.snapshot.as_ref().map(NodeListSnapshot::from),
            latency_ms: self.latency_ms,
            online: self.online,
        }
    }

    pub(super) fn overview_node(&self) -> OverviewNode<'_> {
        OverviewNode {
            online: self.online,
            latency_ms: self.latency_ms,
            snapshot: self.snapshot.as_ref(),
        }
    }

    pub(super) fn prometheus_node(&self) -> PrometheusNode<'_> {
        PrometheusNode {
            identity: &self.identity,
            snapshot: self.snapshot.as_ref(),
            last_seen: self.last_seen,
            latency_ms: self.latency_ms,
            online: self.online,
        }
    }
}

impl AlertStatusView for NodeEntry {
    fn node_id(&self) -> &str {
        &self.identity.node_id
    }

    fn node_label(&self) -> &str {
        &self.identity.node_label
    }

    fn tags(&self) -> &[String] {
        &self.identity.tags
    }

    fn snapshot(&self) -> Option<&NodeSnapshot> {
        self.snapshot.as_ref()
    }

    fn last_seen(&self) -> Option<DateTime<Utc>> {
        self.last_seen
    }

    fn latency_ms(&self) -> Option<u64> {
        self.latency_ms
    }

    fn online(&self) -> bool {
        self.online
    }
}

type GeoIpFields = (Option<Arc<str>>, Option<Arc<str>>, Option<f64>, Option<f64>);

#[allow(clippy::type_complexity)]
pub(super) fn geoip_fields_from_location(
    geoip: Option<&GeoIpLocation>,
    string_pool: &crate::string_pool::StringPool,
) -> GeoIpFields {
    match geoip {
        Some(location) => (
            Some(string_pool.intern(&location.country)),
            location.city.as_ref().map(|city| string_pool.intern(city)),
            location.latitude,
            location.longitude,
        ),
        None => (None, None, None, None),
    }
}
