//! Bounded diagnostic logs: each mutation and eviction completes before releasing the lock.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;

use chrono::{DateTime, Utc};
use tokio::sync::Mutex;

use nodelite_proto::{
    AgentLogEntry, AgentLogsConfig, truncate_to_byte_boundary, validate_identifier,
};

const MAX_LOGS_PER_NODE: usize = 200;
const MAX_BATCH_ENTRIES: usize = 64;
const MAX_LOG_MESSAGE_BYTES: usize = 512;
// Include spare deque slots, hash buckets and allocator overhead, not just serialized text.
const ESTIMATED_LOG_ENTRY_OVERHEAD_BYTES: usize = 512;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RecordResult {
    pub accepted: usize,
    pub dropped_batch_cap: usize,
    pub dropped_sanitize: usize,
    pub evicted_global_budget: usize,
    pub evicted_per_node: usize,
}

impl RecordResult {
    pub fn total_dropped(&self) -> usize {
        self.dropped_batch_cap
            .saturating_add(self.dropped_sanitize)
            .saturating_add(self.evicted_global_budget)
            .saturating_add(self.evicted_per_node)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AgentLogStats {
    pub nodes: usize,
    pub entries: usize,
    pub estimated_bytes: usize,
    pub max_entries: usize,
    pub max_estimated_bytes: usize,
    pub evicted_global_budget_total: u64,
    pub evicted_per_node_total: u64,
    pub dropped_total: u64,
}

#[derive(Clone)]
pub struct AgentLogStore {
    inner: Arc<Mutex<LogState>>,
    limits: AgentLogsConfig,
    pub(crate) lifecycle_lock: Arc<Mutex<()>>,
}

#[derive(Default)]
struct LogState {
    buffers: HashMap<String, VecDeque<StoredAgentLogEntry>>,
    entries: usize,
    estimated_bytes: usize,
    next_sequence: u64,
    evicted_global_budget: u64,
    evicted_per_node: u64,
    dropped: u64,
}

struct StoredAgentLogEntry {
    entry: AgentLogEntry,
    sequence: u64,
    estimated_bytes: usize,
}

impl Default for AgentLogStore {
    fn default() -> Self {
        Self::new()
    }
}

impl AgentLogStore {
    pub fn new() -> Self {
        Self::with_limits(AgentLogsConfig::default())
    }

    pub fn with_limits(limits: AgentLogsConfig) -> Self {
        Self {
            inner: Arc::new(Mutex::new(LogState::default())),
            limits,
            lifecycle_lock: Arc::new(Mutex::new(())),
        }
    }

    pub async fn record_entries(&self, node_id: &str, entries: Vec<AgentLogEntry>) -> RecordResult {
        let mut result = RecordResult {
            dropped_batch_cap: entries.len().saturating_sub(MAX_BATCH_ENTRIES),
            ..Default::default()
        };
        let valid_node = validate_identifier("agent_logs.node_id", node_id).is_ok();
        let mut state = self.inner.lock().await;
        // There are no suspension points between insertion and eviction, so cancellation
        // cannot leave uncharged entries or a batch above either memory budget.
        for entry in entries.into_iter().take(MAX_BATCH_ENTRIES) {
            let Some(entry) = sanitize_entry(entry).filter(|_| valid_node) else {
                result.dropped_sanitize += 1;
                continue;
            };
            state.push_entry(node_id, entry);
            result.accepted += 1;
            while state
                .buffers
                .get(node_id)
                .is_some_and(|buffer| buffer.len() > MAX_LOGS_PER_NODE)
            {
                state.pop_front(node_id);
                result.evicted_per_node += 1;
            }
            result.evicted_global_budget += state.enforce_global_budget(self.limits);
        }
        state.evicted_global_budget = state
            .evicted_global_budget
            .saturating_add(result.evicted_global_budget as u64);
        state.evicted_per_node = state
            .evicted_per_node
            .saturating_add(result.evicted_per_node as u64);
        state.dropped = state
            .dropped
            .saturating_add((result.dropped_batch_cap + result.dropped_sanitize) as u64);
        result
    }

    pub async fn list(&self, node_id: &str, limit: usize) -> Vec<AgentLogEntry> {
        let state = self.inner.lock().await;
        let Some(buffer) = state.buffers.get(node_id) else {
            return Vec::new();
        };
        let start = buffer
            .len()
            .saturating_sub(limit.clamp(1, MAX_LOGS_PER_NODE));
        buffer
            .iter()
            .skip(start)
            .map(|stored| stored.entry.clone())
            .collect()
    }

    pub async fn forget_missing(&self, live_node_ids: &[String]) -> usize {
        let live: HashSet<&str> = live_node_ids.iter().map(String::as_str).collect();
        let mut state = self.inner.lock().await;
        let removed_nodes = state
            .buffers
            .keys()
            .filter(|node_id| !live.contains(node_id.as_str()))
            .cloned()
            .collect::<Vec<_>>();
        for node_id in &removed_nodes {
            state.remove_node(node_id);
        }
        state.buffers.shrink_to_fit();
        removed_nodes.len()
    }

    pub async fn stats(&self) -> AgentLogStats {
        let state = self.inner.lock().await;
        AgentLogStats {
            nodes: state.buffers.len(),
            entries: state.entries,
            estimated_bytes: state.estimated_bytes,
            max_entries: self.limits.max_entries,
            max_estimated_bytes: self.limits.max_estimated_bytes,
            evicted_global_budget_total: state.evicted_global_budget,
            evicted_per_node_total: state.evicted_per_node,
            dropped_total: state.dropped,
        }
    }
}

impl LogState {
    fn push_entry(&mut self, node_id: &str, entry: AgentLogEntry) {
        let estimated_bytes = estimate_entry_bytes(node_id, &entry);
        let stored = StoredAgentLogEntry {
            entry,
            sequence: self.next_sequence,
            estimated_bytes,
        };
        self.next_sequence = self.next_sequence.saturating_add(1);
        self.buffers
            .entry(node_id.to_string())
            .or_default()
            .push_back(stored);
        self.entries += 1;
        self.estimated_bytes += estimated_bytes;
    }

    fn enforce_global_budget(&mut self, limits: AgentLogsConfig) -> usize {
        let mut evicted = 0;
        while self.entries > limits.max_entries || self.estimated_bytes > limits.max_estimated_bytes
        {
            let oldest = self
                .buffers
                .iter()
                .filter_map(|(node_id, buffer)| {
                    buffer.front().map(|entry| (node_id, entry.sequence))
                })
                .min_by_key(|(_, sequence)| *sequence)
                .map(|(node_id, _)| node_id.clone());
            let Some(node_id) = oldest else {
                break;
            };
            self.pop_front(&node_id);
            evicted += 1;
        }
        evicted
    }

    fn pop_front(&mut self, node_id: &str) {
        let Some(buffer) = self.buffers.get_mut(node_id) else {
            return;
        };
        if let Some(entry) = buffer.pop_front() {
            self.entries -= 1;
            self.estimated_bytes -= entry.estimated_bytes;
        }
        if buffer.is_empty() {
            self.buffers.remove(node_id);
        } else if buffer.capacity() > buffer.len().saturating_mul(2) {
            buffer.shrink_to_fit();
        }
        if self.buffers.capacity() > self.buffers.len().saturating_mul(2).max(32) {
            self.buffers.shrink_to_fit();
        }
    }

    fn remove_node(&mut self, node_id: &str) {
        if let Some(buffer) = self.buffers.remove(node_id) {
            self.entries -= buffer.len();
            self.estimated_bytes -= buffer
                .iter()
                .map(|entry| entry.estimated_bytes)
                .sum::<usize>();
        }
    }
}

fn sanitize_entry(mut entry: AgentLogEntry) -> Option<AgentLogEntry> {
    let message = entry.message.trim();
    if message.is_empty() {
        return None;
    }
    entry.message = truncate_to_byte_boundary(message, MAX_LOG_MESSAGE_BYTES).to_string();
    entry.occurred_at = DateTime::parse_from_rfc3339(&entry.occurred_at)
        .map(|timestamp| timestamp.to_rfc3339())
        .unwrap_or_else(|_| Utc::now().to_rfc3339());
    Some(entry)
}

fn estimate_entry_bytes(node_id: &str, entry: &AgentLogEntry) -> usize {
    ESTIMATED_LOG_ENTRY_OVERHEAD_BYTES
        .saturating_add(node_id.len())
        .saturating_add(entry.occurred_at.capacity())
        .saturating_add(entry.message.capacity())
}

#[cfg(test)]
mod tests;
