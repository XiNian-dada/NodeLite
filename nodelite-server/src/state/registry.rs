//! 节点运行态注册表与会话生命周期。

use std::cmp::Ordering;
use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::{RwLock, RwLockReadGuard, RwLockWriteGuard};
use std::time::Duration;

use chrono::{DateTime, Utc};
#[cfg(test)]
use nodelite_proto::DiskUsage;
use nodelite_proto::{
    AlertRuleConfig, GeoIpLocation, InspectionConfig, MetricsConfig, NodeIdentity,
    NodeListIdentity, NodeListItem, NodeListSnapshot, NodeSnapshot, NodeStatus, OverviewData,
};

use super::overview::{OverviewNode, build_overview_from_iter};
use super::session_control::SessionControlHandle;
use crate::ServerReadiness;
use crate::alerts::{
    AlertStatusView, EvaluatedRule, InspectionReport,
    build_inspection_report as build_alert_inspection_report, evaluate_rules,
};
use crate::handlers::metrics_exporter::{PrometheusNode, render_prometheus_metrics_from_iter};

const REGISTRY_SHARD_COUNT: usize = 32;

#[derive(Debug)]
pub(super) struct Registry {
    shards: Vec<RwLock<RegistryShard>>,
}

impl Default for Registry {
    fn default() -> Self {
        Self {
            shards: (0..REGISTRY_SHARD_COUNT)
                .map(|_| RwLock::new(RegistryShard::default()))
                .collect(),
        }
    }
}

#[derive(Debug, Default)]
struct RegistryShard {
    nodes: HashMap<String, NodeEntry>,
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
    location_override_country: Option<String>,
    location_override_city: Option<String>,
    location_override_latitude: Option<f64>,
    location_override_longitude: Option<f64>,
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
        location_override: Option<GeoIpLocation>,
        now: DateTime<Utc>,
    ) -> Self {
        let (geoip_country, geoip_city, geoip_latitude, geoip_longitude) =
            geoip_fields_from_location(geoip.as_ref());
        let (
            location_override_country,
            location_override_city,
            location_override_latitude,
            location_override_longitude,
        ) = geoip_fields_from_location(location_override.as_ref());
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

    fn from_restored_status(mut status: NodeStatus) -> Self {
        status.online = false;
        Self {
            identity: status.identity,
            remote_ip: status.remote_ip,
            geoip_country: status.geoip_country,
            geoip_city: status.geoip_city,
            geoip_latitude: status.geoip_latitude,
            geoip_longitude: status.geoip_longitude,
            location_override_country: status.location_override_country,
            location_override_city: status.location_override_city,
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

    fn register_session(
        &mut self,
        session_id: u64,
        identity: NodeIdentity,
        remote_ip: Option<String>,
        geoip: Option<GeoIpLocation>,
        location_override: Option<GeoIpLocation>,
        now: DateTime<Utc>,
    ) {
        let (geoip_country, geoip_city, geoip_latitude, geoip_longitude) =
            geoip_fields_from_location(geoip.as_ref());
        let (
            location_override_country,
            location_override_city,
            location_override_latitude,
            location_override_longitude,
        ) = geoip_fields_from_location(location_override.as_ref());
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

    fn to_status(&self) -> NodeStatus {
        NodeStatus {
            identity: self.identity.clone(),
            remote_ip: self.remote_ip.clone(),
            geoip_country: self.geoip_country.clone(),
            geoip_city: self.geoip_city.clone(),
            geoip_latitude: self.geoip_latitude,
            geoip_longitude: self.geoip_longitude,
            location_override_country: self.location_override_country.clone(),
            location_override_city: self.location_override_city.clone(),
            location_override_latitude: self.location_override_latitude,
            location_override_longitude: self.location_override_longitude,
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
            location_override_country: self.location_override_country.clone(),
            location_override_city: self.location_override_city.clone(),
            location_override_latitude: self.location_override_latitude,
            location_override_longitude: self.location_override_longitude,
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
        &self,
        session_id: u64,
        identity: NodeIdentity,
        remote_ip: Option<String>,
        geoip: Option<GeoIpLocation>,
        location_override: Option<GeoIpLocation>,
        now: DateTime<Utc>,
    ) {
        let node_id = identity.node_id.clone();
        let mut shard = write_lock(self.shard_for(&node_id));
        if let Some(entry) = shard.nodes.get_mut(&node_id) {
            entry.register_session(
                session_id,
                identity,
                remote_ip,
                geoip,
                location_override,
                now,
            );
        } else {
            shard.nodes.insert(
                node_id,
                NodeEntry::new(
                    session_id,
                    identity,
                    remote_ip,
                    geoip,
                    location_override,
                    now,
                ),
            );
        }
    }

    pub(super) fn update_snapshot(
        &self,
        node_id: &str,
        session_id: u64,
        snapshot: NodeSnapshot,
        now: DateTime<Utc>,
    ) -> Option<NodeStatus> {
        let mut shard = write_lock(self.shard_for(node_id));
        let entry = shard.nodes.get_mut(node_id)?;
        if entry.active_session_id != Some(session_id) {
            return None;
        }

        entry.snapshot = Some(snapshot);
        entry.last_seen = Some(now);
        entry.online = true;
        Some(entry.to_status())
    }

    pub(super) fn update_latency(
        &self,
        node_id: &str,
        session_id: u64,
        latency_ms: u64,
        now: DateTime<Utc>,
    ) -> bool {
        let mut shard = write_lock(self.shard_for(node_id));
        let Some(entry) = shard.nodes.get_mut(node_id) else {
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

    pub(super) fn mark_disconnected(&self, node_id: &str, session_id: u64) -> bool {
        let mut shard = write_lock(self.shard_for(node_id));
        let Some(entry) = shard.nodes.get_mut(node_id) else {
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
        &self,
        node_id: &str,
        session_id: u64,
        control: SessionControlHandle,
    ) -> bool {
        let mut shard = write_lock(self.shard_for(node_id));
        let Some(entry) = shard.nodes.get_mut(node_id) else {
            return false;
        };
        if entry.active_session_id != Some(session_id) {
            return false;
        }

        entry.control = Some(control);
        true
    }

    pub(super) fn mark_stale(&self, threshold: Duration, now: DateTime<Utc>) -> usize {
        let mut marked = 0;

        for shard in &self.shards {
            let mut shard = write_lock(shard);
            for entry in shard.nodes.values_mut() {
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
        }

        marked
    }

    pub(super) fn is_current_session(&self, node_id: &str, session_id: u64) -> bool {
        read_lock(self.shard_for(node_id))
            .nodes
            .get(node_id)
            .and_then(|entry| entry.active_session_id)
            == Some(session_id)
    }

    pub(super) fn list_statuses(&self) -> Vec<NodeStatus> {
        let shards = self.read_all_shards();
        sorted_entries(&shards)
            .into_iter()
            .map(NodeEntry::to_status)
            .collect()
    }

    pub(super) fn list_node_summaries(&self) -> Vec<NodeListItem> {
        let shards = self.read_all_shards();
        sorted_entries(&shards)
            .into_iter()
            .map(NodeEntry::to_summary)
            .collect()
    }

    pub(super) fn browser_view_with_revision<R>(
        &self,
        load_revision: impl FnOnce() -> R,
    ) -> (Vec<NodeListItem>, OverviewData, R) {
        let shards = self.read_all_shards();
        let nodes = sorted_entries(&shards)
            .into_iter()
            .map(NodeEntry::to_summary)
            .collect();
        let overview = overview_from_shards(&shards);
        let revision = load_revision();
        (nodes, overview, revision)
    }

    pub(super) fn evaluate_alert_rules(
        &self,
        rules: &[AlertRuleConfig],
        now: DateTime<Utc>,
    ) -> Vec<EvaluatedRule> {
        let shards = self.read_all_shards();
        evaluate_rules(rules, sorted_entries(&shards), now)
    }

    pub(super) fn build_alert_inspection_report(
        &self,
        inspection: &InspectionConfig,
        now: DateTime<Utc>,
    ) -> InspectionReport {
        let shards = self.read_all_shards();
        build_alert_inspection_report(inspection, sorted_entries(&shards), now)
    }

    pub(super) fn get_status(&self, node_id: &str) -> Option<NodeStatus> {
        read_lock(self.shard_for(node_id))
            .nodes
            .get(node_id)
            .map(NodeEntry::to_status)
    }

    pub(super) fn geoip_refresh_candidates(&self) -> Vec<(String, String)> {
        let shards = self.read_all_shards();
        sorted_entries(&shards)
            .into_iter()
            .filter_map(|entry| {
                if !entry.online || entry.active_session_id.is_none() {
                    return None;
                }
                entry
                    .remote_ip
                    .as_ref()
                    .map(|remote_ip| (entry.identity.node_id.clone(), remote_ip.clone()))
            })
            .collect()
    }

    pub(super) fn update_geoip(
        &self,
        node_id: &str,
        expected_remote_ip: &str,
        geoip: GeoIpLocation,
    ) -> bool {
        let mut shard = write_lock(self.shard_for(node_id));
        let Some(entry) = shard.nodes.get_mut(node_id) else {
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

    pub(super) fn update_location_override(
        &self,
        node_id: &str,
        location_override: Option<GeoIpLocation>,
    ) -> bool {
        let mut shard = write_lock(self.shard_for(node_id));
        let Some(entry) = shard.nodes.get_mut(node_id) else {
            return false;
        };
        let (
            location_override_country,
            location_override_city,
            location_override_latitude,
            location_override_longitude,
        ) = geoip_fields_from_location(location_override.as_ref());
        if entry.location_override_country == location_override_country
            && entry.location_override_city == location_override_city
            && entry.location_override_latitude == location_override_latitude
            && entry.location_override_longitude == location_override_longitude
        {
            return false;
        }

        entry.location_override_country = location_override_country;
        entry.location_override_city = location_override_city;
        entry.location_override_latitude = location_override_latitude;
        entry.location_override_longitude = location_override_longitude;
        true
    }

    pub(super) fn session_control(&self, node_id: &str) -> Option<SessionControlHandle> {
        let shard = read_lock(self.shard_for(node_id));
        let entry = shard.nodes.get(node_id)?;
        if entry.active_session_id.is_none() || !entry.online {
            return None;
        }
        entry.control.clone()
    }

    pub(super) fn overview(&self) -> OverviewData {
        let shards = self.read_all_shards();
        overview_from_shards(&shards)
    }

    pub(super) fn render_metrics_body(
        &self,
        readiness: &ServerReadiness,
        metrics_config: MetricsConfig,
    ) -> String {
        let shards = self.read_all_shards();
        let overview = overview_from_shards(&shards);
        let entries = sorted_entries(&shards);
        render_prometheus_metrics_from_iter(
            readiness,
            entries.into_iter().map(NodeEntry::prometheus_node),
            &overview,
            metrics_config,
        )
    }

    pub(super) fn disk_entries_total(&self) -> u64 {
        self.shards
            .iter()
            .map(|shard| {
                read_lock(shard)
                    .nodes
                    .values()
                    .filter_map(|entry| entry.snapshot.as_ref())
                    .map(|snapshot| snapshot.disks.len() as u64)
                    .sum::<u64>()
            })
            .sum()
    }

    pub(super) fn restore_statuses(&self, statuses: Vec<NodeStatus>) {
        for shard in &self.shards {
            write_lock(shard).nodes.clear();
        }
        for status in statuses {
            let node_id = status.identity.node_id.clone();
            write_lock(self.shard_for(&node_id))
                .nodes
                .insert(node_id, NodeEntry::from_restored_status(status));
        }
    }

    fn read_all_shards(&self) -> Vec<RwLockReadGuard<'_, RegistryShard>> {
        self.shards.iter().map(read_lock).collect()
    }

    fn shard_for(&self, node_id: &str) -> &RwLock<RegistryShard> {
        &self.shards[shard_index(node_id)]
    }

    #[cfg(test)]
    pub(super) fn shard_index_for_test(node_id: &str) -> usize {
        shard_index(node_id)
    }

    #[cfg(test)]
    pub(super) fn shard_count_for_test() -> usize {
        REGISTRY_SHARD_COUNT
    }

    #[cfg(test)]
    pub(super) fn nodes_per_shard_for_test(&self) -> Vec<usize> {
        self.shards
            .iter()
            .map(|shard| read_lock(shard).nodes.len())
            .collect()
    }

    #[cfg(test)]
    pub(super) fn shard_is_read_locked_for_test(&self, node_id: &str) -> bool {
        self.shard_for(node_id).try_write().is_err()
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
}

fn sorted_entries<'a>(shards: &'a [RwLockReadGuard<'_, RegistryShard>]) -> Vec<&'a NodeEntry> {
    let mut entries = shards
        .iter()
        .flat_map(|shard| shard.nodes.values())
        .collect::<Vec<_>>();
    entries.sort_by(|left, right| compare_node_entries(left, right));
    entries
}

fn compare_node_entries(left: &NodeEntry, right: &NodeEntry) -> Ordering {
    left.identity
        .node_label
        .cmp(&right.identity.node_label)
        .then_with(|| left.identity.node_id.cmp(&right.identity.node_id))
}

fn overview_from_shards(shards: &[RwLockReadGuard<'_, RegistryShard>]) -> OverviewData {
    build_overview_from_iter(
        shards
            .iter()
            .flat_map(|shard| shard.nodes.values().map(NodeEntry::overview_node)),
    )
}

fn shard_index(node_id: &str) -> usize {
    let mut hasher = DefaultHasher::new();
    node_id.hash(&mut hasher);
    (hasher.finish() as usize) % REGISTRY_SHARD_COUNT
}

fn read_lock<T>(lock: &RwLock<T>) -> RwLockReadGuard<'_, T> {
    lock.read().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn write_lock<T>(lock: &RwLock<T>) -> RwLockWriteGuard<'_, T> {
    lock.write()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
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
