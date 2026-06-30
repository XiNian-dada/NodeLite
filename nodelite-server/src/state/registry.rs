//! 节点运行态注册表与会话生命周期。

use std::cmp::Ordering;
use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard};
use std::time::Duration;

use chrono::{DateTime, Utc};
use nodelite_proto::{
    AlertRuleConfig, GeoIpLocation, InspectionConfig, MetricsConfig, NodeIdentity, NodeListItem,
    NodeListItemView, NodeSnapshot, NodeStatus, OverviewData,
};

use super::overview::build_overview_from_iter;
use super::session_control::SessionControlHandle;
use crate::ServerReadiness;
use crate::alerts::{
    EvaluatedRule, InspectionReport, build_inspection_report as build_alert_inspection_report,
    evaluate_rules,
};
use crate::handlers::metrics_routes::render_prometheus_metrics_from_iter;

mod entry;
#[cfg(test)]
mod heap_estimate;

use entry::{NodeEntry, geoip_fields_from_location};

const REGISTRY_SHARD_COUNT: usize = 32;

#[derive(Debug)]
pub(super) struct Registry {
    shards: Vec<RwLock<RegistryShard>>,
    string_pool: Arc<crate::string_pool::StringPool>,
}

impl Default for Registry {
    fn default() -> Self {
        Self {
            shards: (0..REGISTRY_SHARD_COUNT)
                .map(|_| RwLock::new(RegistryShard::default()))
                .collect(),
            string_pool: Arc::new(crate::string_pool::StringPool::new()),
        }
    }
}

#[derive(Debug, Default)]
struct RegistryShard {
    nodes: HashMap<String, NodeEntry>,
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
                &self.string_pool,
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
                    &self.string_pool,
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

    #[cfg(test)]
    pub(super) fn list_node_summaries(&self) -> Vec<NodeListItem> {
        let shards = self.read_all_shards();
        sorted_entries(&shards)
            .into_iter()
            .map(NodeEntry::to_summary)
            .collect()
    }

    /// 零拷贝版本的 `list_node_summaries` (Phase 3.2 优化)。
    ///
    /// 返回 `NodeListItemView` 避免从 `Arc<str>` 克隆字符串,减少 API 响应延迟。
    pub(super) fn list_node_summaries_view(&self) -> Vec<NodeListItemView> {
        let shards = self.read_all_shards();
        sorted_entries(&shards)
            .into_iter()
            .map(NodeEntry::to_summary_view)
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

        let geoip_country = Some(self.string_pool.intern(&geoip.country));
        let geoip_city = geoip
            .city
            .as_ref()
            .map(|city| self.string_pool.intern(city));
        let geoip_latitude = geoip.latitude;
        let geoip_longitude = geoip.longitude;
        if entry.geoip_country.as_ref().map(|s| s.as_ref())
            == geoip_country.as_ref().map(|s| s.as_ref())
            && entry.geoip_city.as_ref().map(|s| s.as_ref())
                == geoip_city.as_ref().map(|s| s.as_ref())
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
        ) = geoip_fields_from_location(location_override.as_ref(), &self.string_pool);
        if entry.location_override_country.as_ref().map(|s| s.as_ref())
            == location_override_country.as_ref().map(|s| s.as_ref())
            && entry.location_override_city.as_ref().map(|s| s.as_ref())
                == location_override_city.as_ref().map(|s| s.as_ref())
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
            Some(self.string_pool.len()),
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
            write_lock(self.shard_for(&node_id)).nodes.insert(
                node_id,
                NodeEntry::from_restored_status(status, &self.string_pool),
            );
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
    ) -> (
        heap_estimate::RetainedHeapEstimate,
        heap_estimate::RetainedHeapEstimate,
    ) {
        heap_estimate::retained_heap_estimates_for_status(status)
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
