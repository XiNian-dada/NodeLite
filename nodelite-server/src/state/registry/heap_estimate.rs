use std::sync::Arc;

use nodelite_proto::{
    DiskUsage, NodeIdentity, NodeListIdentity, NodeListItem, NodeSnapshot, NodeStatus,
};

use super::entry::NodeEntry;

#[derive(Debug, Clone, Copy, Default)]
pub(in crate::state) struct RetainedHeapEstimate {
    pub(in crate::state) bytes: usize,
    pub(in crate::state) allocations: usize,
}

impl std::ops::Add for RetainedHeapEstimate {
    type Output = Self;

    fn add(self, other: Self) -> Self {
        Self {
            bytes: self.bytes + other.bytes,
            allocations: self.allocations + other.allocations,
        }
    }
}

pub(super) fn retained_heap_estimates_for_status(
    status: NodeStatus,
) -> (RetainedHeapEstimate, RetainedHeapEstimate) {
    let string_pool = crate::string_pool::StringPool::new();
    let previous_summary = NodeListItem::from(&status);
    let previous =
        node_status_heap_estimate(&status) + node_list_item_heap_estimate(&previous_summary);
    let runtime = node_entry_heap_estimate(&NodeEntry::from_restored_status(status, &string_pool));
    (runtime, previous)
}

fn node_entry_heap_estimate(entry: &NodeEntry) -> RetainedHeapEstimate {
    node_identity_heap_estimate(&entry.identity)
        + option_string_heap_estimate(&entry.remote_ip)
        + option_arc_str_heap_estimate(&entry.geoip_country)
        + option_arc_str_heap_estimate(&entry.geoip_city)
        + entry
            .snapshot
            .as_ref()
            .map(node_snapshot_heap_estimate)
            .unwrap_or_default()
}

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

fn node_list_item_heap_estimate(item: &NodeListItem) -> RetainedHeapEstimate {
    node_list_identity_heap_estimate(&item.identity)
        + option_string_heap_estimate(&item.geoip_country)
        + option_string_heap_estimate(&item.geoip_city)
}

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

fn node_list_identity_heap_estimate(identity: &NodeListIdentity) -> RetainedHeapEstimate {
    string_heap_estimate(&identity.node_id)
        + string_heap_estimate(&identity.node_label)
        + string_heap_estimate(&identity.hostname)
        + string_vec_heap_estimate(&identity.tags)
}

fn node_snapshot_heap_estimate(snapshot: &NodeSnapshot) -> RetainedHeapEstimate {
    vec_buffer_heap_estimate::<DiskUsage>(snapshot.disks.capacity())
        + snapshot
            .disks
            .iter()
            .map(disk_usage_heap_estimate)
            .fold(RetainedHeapEstimate::default(), |total, next| total + next)
}

fn disk_usage_heap_estimate(disk: &DiskUsage) -> RetainedHeapEstimate {
    string_heap_estimate(&disk.device)
        + string_heap_estimate(&disk.mount_point)
        + string_heap_estimate(&disk.fs_type)
}

fn string_vec_heap_estimate(values: &[String]) -> RetainedHeapEstimate {
    vec_buffer_heap_estimate::<String>(values.len())
        + values
            .iter()
            .map(string_heap_estimate)
            .fold(RetainedHeapEstimate::default(), |total, next| total + next)
}

fn option_string_heap_estimate(value: &Option<String>) -> RetainedHeapEstimate {
    value.as_ref().map(string_heap_estimate).unwrap_or_default()
}

fn option_arc_str_heap_estimate(value: &Option<Arc<str>>) -> RetainedHeapEstimate {
    value
        .as_ref()
        .map(arc_str_heap_estimate)
        .unwrap_or_default()
}

fn string_heap_estimate(value: &String) -> RetainedHeapEstimate {
    RetainedHeapEstimate {
        bytes: value.capacity(),
        allocations: usize::from(value.capacity() > 0),
    }
}

fn arc_str_heap_estimate(value: &Arc<str>) -> RetainedHeapEstimate {
    RetainedHeapEstimate {
        bytes: std::mem::size_of::<usize>() * 2 + value.len(),
        allocations: 1,
    }
}

fn vec_buffer_heap_estimate<T>(capacity: usize) -> RetainedHeapEstimate {
    RetainedHeapEstimate {
        bytes: capacity * std::mem::size_of::<T>(),
        allocations: usize::from(capacity > 0),
    }
}
