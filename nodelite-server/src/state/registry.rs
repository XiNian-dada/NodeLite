//! 节点运行态注册表与会话生命周期。

use std::collections::HashMap;
use std::time::Duration;

use chrono::{DateTime, Utc};
#[cfg(test)]
use nodelite_proto::DiskUsage;
use nodelite_proto::{
    GeoIpLocation, MetricsConfig, NodeIdentity, NodeListIdentity, NodeListItem, NodeListSnapshot,
    NodeSnapshot, NodeStatus, OverviewData,
};

use super::overview::{OverviewNode, build_overview_from_iter};
use super::session_control::SessionControlHandle;
use crate::ServerReadiness;
use crate::handlers::metrics_exporter::{PrometheusNode, render_prometheus_metrics_from_iter};

#[derive(Debug, Default)]
pub(super) struct Registry {
    nodes: HashMap<String, NodeEntry>,
    sorted_node_ids: Vec<String>,
}

/// 单节点的运行态条目。外部响应模型只在 API / snapshot 边界按需组装。
#[derive(Debug, Clone)]
struct NodeEntry {
    identity: NodeIdentity,
    remote_ip: Option<String>,
    geoip_country: Option<String>,
    geoip_city: Option<String>,
    geoip_latitude: Option<f64>,
    geoip_longitude: Option<f64>,
    snapshot: Option<NodeSnapshot>,
    last_seen: Option<DateTime<Utc>>,
    latency_ms: Option<u64>,
    online: bool,
    active_session_id: Option<u64>,
    control: Option<SessionControlHandle>,
}

impl NodeEntry {
    fn new(
        session_id: u64,
        identity: NodeIdentity,
        remote_ip: Option<String>,
        geoip: Option<GeoIpLocation>,
        now: DateTime<Utc>,
    ) -> Self {
        let (geoip_country, geoip_city, geoip_latitude, geoip_longitude) =
            geoip_fields_from_location(geoip.as_ref());
        Self {
            identity,
            remote_ip,
            geoip_country,
            geoip_city,
            geoip_latitude,
            geoip_longitude,
            snapshot: None,
            last_seen: Some(now),
            latency_ms: None,
            online: true,
            active_session_id: Some(session_id),
            control: None,
        }
    }

    fn from_restored_status(mut status: NodeStatus) -> Self {
        status.online = false;
        Self {
            identity: status.identity,
            remote_ip: status.remote_ip,
            geoip_country: status.geoip_country,
            geoip_city: status.geoip_city,
            geoip_latitude: status.geoip_latitude,
            geoip_longitude: status.geoip_longitude,
            snapshot: status.snapshot,
            last_seen: status.last_seen,
            latency_ms: status.latency_ms,
            online: false,
            active_session_id: None,
            control: None,
        }
    }

    fn register_session(
        &mut self,
        session_id: u64,
        identity: NodeIdentity,
        remote_ip: Option<String>,
        geoip: Option<GeoIpLocation>,
        now: DateTime<Utc>,
    ) {
        let (geoip_country, geoip_city, geoip_latitude, geoip_longitude) =
            geoip_fields_from_location(geoip.as_ref());
        self.identity = identity;
        self.remote_ip = remote_ip;
        self.geoip_country = geoip_country;
        self.geoip_city = geoip_city;
        self.geoip_latitude = geoip_latitude;
        self.geoip_longitude = geoip_longitude;
        self.online = true;
        self.last_seen = Some(now);
        self.latency_ms = None;
        self.active_session_id = Some(session_id);
        self.control = None;
    }

    fn to_status(&self) -> NodeStatus {
        NodeStatus {
            identity: self.identity.clone(),
            remote_ip: self.remote_ip.clone(),
            geoip_country: self.geoip_country.clone(),
            geoip_city: self.geoip_city.clone(),
            geoip_latitude: self.geoip_latitude,
            geoip_longitude: self.geoip_longitude,
            snapshot: self.snapshot.clone(),
            last_seen: self.last_seen,
            latency_ms: self.latency_ms,
            online: self.online,
        }
    }

    fn to_summary(&self) -> NodeListItem {
        NodeListItem {
            identity: NodeListIdentity::from(&self.identity),
            geoip_country: self.geoip_country.clone(),
            geoip_city: self.geoip_city.clone(),
            geoip_latitude: self.geoip_latitude,
            geoip_longitude: self.geoip_longitude,
            snapshot: self.snapshot.as_ref().map(NodeListSnapshot::from),
            latency_ms: self.latency_ms,
            online: self.online,
        }
    }

    fn overview_node(&self) -> OverviewNode<'_> {
        OverviewNode {
            online: self.online,
            latency_ms: self.latency_ms,
            snapshot: self.snapshot.as_ref(),
        }
    }

    fn prometheus_node(&self) -> PrometheusNode<'_> {
        PrometheusNode {
            identity: &self.identity,
            snapshot: self.snapshot.as_ref(),
            last_seen: self.last_seen,
            latency_ms: self.latency_ms,
            online: self.online,
        }
    }
}

fn geoip_fields_from_location(
    geoip: Option<&GeoIpLocation>,
) -> (Option<String>, Option<String>, Option<f64>, Option<f64>) {
    (
        geoip.map(|location| location.country.clone()),
        geoip.and_then(|location| location.city.clone()),
        geoip.and_then(|location| location.latitude),
        geoip.and_then(|location| location.longitude),
    )
}

impl Registry {
    pub(super) fn register_node(
        &mut self,
        session_id: u64,
        identity: NodeIdentity,
        remote_ip: Option<String>,
        geoip: Option<GeoIpLocation>,
        now: DateTime<Utc>,
    ) {
        let node_id = identity.node_id.clone();
        if let Some(entry) = self.nodes.get_mut(&node_id) {
            entry.register_session(session_id, identity, remote_ip, geoip, now);
        } else {
            self.nodes.insert(
                node_id.clone(),
                NodeEntry::new(session_id, identity, remote_ip, geoip, now),
            );
            self.sorted_node_ids.push(node_id);
        }
        self.resort_node_ids();
    }

    pub(super) fn update_snapshot(
        &mut self,
        node_id: &str,
        session_id: u64,
        snapshot: NodeSnapshot,
        now: DateTime<Utc>,
    ) -> Option<NodeStatus> {
        let entry = self.nodes.get_mut(node_id)?;
        if entry.active_session_id != Some(session_id) {
            return None;
        }

        entry.snapshot = Some(snapshot);
        entry.last_seen = Some(now);
        entry.online = true;
        Some(entry.to_status())
    }

    pub(super) fn update_latency(
        &mut self,
        node_id: &str,
        session_id: u64,
        latency_ms: u64,
        now: DateTime<Utc>,
    ) -> bool {
        let Some(entry) = self.nodes.get_mut(node_id) else {
            return false;
        };
        if entry.active_session_id != Some(session_id) {
            return false;
        }

        entry.latency_ms = Some(latency_ms);
        entry.last_seen = Some(now);
        entry.online = true;
        true
    }

    pub(super) fn mark_disconnected(&mut self, node_id: &str, session_id: u64) -> bool {
        let Some(entry) = self.nodes.get_mut(node_id) else {
            return false;
        };
        if entry.active_session_id == Some(session_id) {
            entry.active_session_id = None;
            entry.online = false;
            entry.control = None;
            return true;
        }
        false
    }

    pub(super) fn attach_session_control(
        &mut self,
        node_id: &str,
        session_id: u64,
        control: SessionControlHandle,
    ) -> bool {
        let Some(entry) = self.nodes.get_mut(node_id) else {
            return false;
        };
        if entry.active_session_id != Some(session_id) {
            return false;
        }

        entry.control = Some(control);
        true
    }

    pub(super) fn mark_stale(&mut self, threshold: Duration, now: DateTime<Utc>) -> usize {
        let mut marked = 0;

        for entry in self.nodes.values_mut() {
            let Some(last_seen) = entry.last_seen else {
                continue;
            };
            let Ok(elapsed) = (now - last_seen).to_std() else {
                continue;
            };
            if elapsed >= threshold && entry.online {
                entry.online = false;
                entry.active_session_id = None;
                entry.control = None;
                marked += 1;
            }
        }

        marked
    }

    pub(super) fn is_current_session(&self, node_id: &str, session_id: u64) -> bool {
        self.nodes
            .get(node_id)
            .and_then(|entry| entry.active_session_id)
            == Some(session_id)
    }

    pub(super) fn list_statuses(&self) -> Vec<NodeStatus> {
        self.sorted_node_ids
            .iter()
            .filter_map(|node_id| self.nodes.get(node_id))
            .map(NodeEntry::to_status)
            .collect()
    }

    pub(super) fn list_node_summaries(&self) -> Vec<NodeListItem> {
        self.sorted_node_ids
            .iter()
            .filter_map(|node_id| self.nodes.get(node_id))
            .map(NodeEntry::to_summary)
            .collect()
    }

    pub(super) fn get_status(&self, node_id: &str) -> Option<NodeStatus> {
        self.nodes.get(node_id).map(NodeEntry::to_status)
    }

    pub(super) fn geoip_refresh_candidates(&self) -> Vec<(String, String)> {
        self.sorted_node_ids
            .iter()
            .filter_map(|node_id| {
                let entry = self.nodes.get(node_id)?;
                if !entry.online || entry.active_session_id.is_none() {
                    return None;
                }
                entry
                    .remote_ip
                    .as_ref()
                    .map(|remote_ip| (node_id.clone(), remote_ip.clone()))
            })
            .collect()
    }

    pub(super) fn update_geoip(
        &mut self,
        node_id: &str,
        expected_remote_ip: &str,
        geoip: GeoIpLocation,
    ) -> bool {
        let Some(entry) = self.nodes.get_mut(node_id) else {
            return false;
        };
        if entry.remote_ip.as_deref() != Some(expected_remote_ip) {
            return false;
        }

        let geoip_country = Some(geoip.country);
        let geoip_city = geoip.city;
        let geoip_latitude = geoip.latitude;
        let geoip_longitude = geoip.longitude;
        if entry.geoip_country == geoip_country
            && entry.geoip_city == geoip_city
            && entry.geoip_latitude == geoip_latitude
            && entry.geoip_longitude == geoip_longitude
        {
            return false;
        }

        entry.geoip_country = geoip_country;
        entry.geoip_city = geoip_city;
        entry.geoip_latitude = geoip_latitude;
        entry.geoip_longitude = geoip_longitude;
        true
    }

    pub(super) fn session_control(&self, node_id: &str) -> Option<SessionControlHandle> {
        let entry = self.nodes.get(node_id)?;
        if entry.active_session_id.is_none() || !entry.online {
            return None;
        }
        entry.control.clone()
    }

    pub(super) fn overview(&self) -> OverviewData {
        build_overview_from_iter(self.nodes.values().map(NodeEntry::overview_node))
    }

    pub(super) fn render_metrics_body(
        &self,
        readiness: &ServerReadiness,
        metrics_config: MetricsConfig,
    ) -> String {
        let overview = self.overview();
        render_prometheus_metrics_from_iter(
            readiness,
            self.sorted_node_ids
                .iter()
                .filter_map(|node_id| self.nodes.get(node_id))
                .map(NodeEntry::prometheus_node),
            &overview,
            metrics_config,
        )
    }

    pub(super) fn disk_entries_total(&self) -> u64 {
        self.nodes
            .values()
            .filter_map(|entry| entry.snapshot.as_ref())
            .map(|snapshot| snapshot.disks.len() as u64)
            .sum()
    }

    pub(super) fn restore_statuses(&mut self, statuses: Vec<NodeStatus>) {
        self.nodes.clear();
        self.sorted_node_ids.clear();
        for status in statuses {
            let node_id = status.identity.node_id.clone();
            self.nodes
                .insert(node_id.clone(), NodeEntry::from_restored_status(status));
            self.sorted_node_ids.push(node_id);
        }
        self.resort_node_ids();
    }

    #[cfg(test)]
    pub(super) fn runtime_entry_inline_bytes_for_test() -> usize {
        std::mem::size_of::<NodeEntry>()
    }

    #[cfg(test)]
    pub(super) fn previous_external_model_inline_bytes_for_test() -> usize {
        std::mem::size_of::<NodeStatus>()
            + std::mem::size_of::<NodeListItem>()
            + std::mem::size_of::<Option<u64>>()
            + std::mem::size_of::<Option<SessionControlHandle>>()
    }

    #[cfg(test)]
    pub(super) fn retained_heap_estimates_for_test(
        status: NodeStatus,
    ) -> (RetainedHeapEstimate, RetainedHeapEstimate) {
        let previous_summary = NodeListItem::from(&status);
        let previous =
            node_status_heap_estimate(&status) + node_list_item_heap_estimate(&previous_summary);
        let runtime = node_entry_heap_estimate(&NodeEntry::from_restored_status(status));
        (runtime, previous)
    }

    fn resort_node_ids(&mut self) {
        self.sorted_node_ids.sort_by(|left_id, right_id| {
            let (Some(left), Some(right)) = (self.nodes.get(left_id), self.nodes.get(right_id))
            else {
                return left_id.cmp(right_id);
            };
            left.identity
                .node_label
                .cmp(&right.identity.node_label)
                .then_with(|| left.identity.node_id.cmp(&right.identity.node_id))
        });
    }
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, Default)]
pub(super) struct RetainedHeapEstimate {
    pub(super) bytes: usize,
    pub(super) allocations: usize,
}

#[cfg(test)]
impl std::ops::Add for RetainedHeapEstimate {
    type Output = Self;

    fn add(self, other: Self) -> Self {
        Self {
            bytes: self.bytes + other.bytes,
            allocations: self.allocations + other.allocations,
        }
    }
}

#[cfg(test)]
fn node_entry_heap_estimate(entry: &NodeEntry) -> RetainedHeapEstimate {
    node_identity_heap_estimate(&entry.identity)
        + option_string_heap_estimate(&entry.remote_ip)
        + option_string_heap_estimate(&entry.geoip_country)
        + option_string_heap_estimate(&entry.geoip_city)
        + entry
            .snapshot
            .as_ref()
            .map(node_snapshot_heap_estimate)
            .unwrap_or_default()
}

#[cfg(test)]
fn node_status_heap_estimate(status: &NodeStatus) -> RetainedHeapEstimate {
    node_identity_heap_estimate(&status.identity)
        + option_string_heap_estimate(&status.remote_ip)
        + option_string_heap_estimate(&status.geoip_country)
        + option_string_heap_estimate(&status.geoip_city)
        + status
            .snapshot
            .as_ref()
            .map(node_snapshot_heap_estimate)
            .unwrap_or_default()
}

#[cfg(test)]
fn node_list_item_heap_estimate(item: &NodeListItem) -> RetainedHeapEstimate {
    node_list_identity_heap_estimate(&item.identity)
        + option_string_heap_estimate(&item.geoip_country)
        + option_string_heap_estimate(&item.geoip_city)
}

#[cfg(test)]
fn node_identity_heap_estimate(identity: &NodeIdentity) -> RetainedHeapEstimate {
    string_heap_estimate(&identity.node_id)
        + string_heap_estimate(&identity.node_label)
        + string_heap_estimate(&identity.hostname)
        + string_heap_estimate(&identity.os)
        + option_string_heap_estimate(&identity.kernel_version)
        + option_string_heap_estimate(&identity.cpu_model)
        + string_heap_estimate(&identity.agent_version)
        + string_vec_heap_estimate(&identity.tags)
}

#[cfg(test)]
fn node_list_identity_heap_estimate(identity: &NodeListIdentity) -> RetainedHeapEstimate {
    string_heap_estimate(&identity.node_id)
        + string_heap_estimate(&identity.node_label)
        + string_heap_estimate(&identity.hostname)
        + string_vec_heap_estimate(&identity.tags)
}

#[cfg(test)]
fn node_snapshot_heap_estimate(snapshot: &NodeSnapshot) -> RetainedHeapEstimate {
    vec_buffer_heap_estimate::<DiskUsage>(snapshot.disks.capacity())
        + snapshot
            .disks
            .iter()
            .map(disk_usage_heap_estimate)
            .fold(RetainedHeapEstimate::default(), |total, next| total + next)
}

#[cfg(test)]
fn disk_usage_heap_estimate(disk: &DiskUsage) -> RetainedHeapEstimate {
    string_heap_estimate(&disk.device)
        + string_heap_estimate(&disk.mount_point)
        + string_heap_estimate(&disk.fs_type)
}

#[cfg(test)]
fn string_vec_heap_estimate(values: &[String]) -> RetainedHeapEstimate {
    vec_buffer_heap_estimate::<String>(values.len())
        + values
            .iter()
            .map(string_heap_estimate)
            .fold(RetainedHeapEstimate::default(), |total, next| total + next)
}

#[cfg(test)]
fn option_string_heap_estimate(value: &Option<String>) -> RetainedHeapEstimate {
    value.as_ref().map(string_heap_estimate).unwrap_or_default()
}

#[cfg(test)]
fn string_heap_estimate(value: &String) -> RetainedHeapEstimate {
    RetainedHeapEstimate {
        bytes: value.capacity(),
        allocations: usize::from(value.capacity() > 0),
    }
}

#[cfg(test)]
fn vec_buffer_heap_estimate<T>(capacity: usize) -> RetainedHeapEstimate {
    RetainedHeapEstimate {
        bytes: capacity * std::mem::size_of::<T>(),
        allocations: usize::from(capacity > 0),
    }
}
