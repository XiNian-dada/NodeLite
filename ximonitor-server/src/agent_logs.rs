use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;

use chrono::{DateTime, Utc};
use tokio::sync::RwLock;
use ximonitor_proto::AgentLogEntry;

const MAX_LOGS_PER_NODE: usize = 200;
const MAX_BATCH_ENTRIES: usize = 64;
const MAX_LOG_MESSAGE_BYTES: usize = 512;

/// 最近 Agent 运行日志的内存缓冲。
///
/// 这些日志只用于只读排障视图,不参与持久化。设计目标是:
/// - 每节点保留固定上限,防止异常节点无限吃内存;
/// - 接受 Agent 断线后回补的一小批日志,帮助排查偶发断链/重连问题;
/// - 对消息长度与时间戳做轻量清洗,避免脏数据破坏前端渲染。
#[derive(Clone, Default)]
pub struct AgentLogStore {
    inner: Arc<RwLock<HashMap<String, VecDeque<AgentLogEntry>>>>,
}

impl AgentLogStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// 记录某节点上传的一批日志,返回实际接收的条数。
    pub async fn record_entries(&self, node_id: &str, entries: Vec<AgentLogEntry>) -> usize {
        let mut guard = self.inner.write().await;
        let buffer = guard.entry(node_id.to_string()).or_default();
        let mut accepted = 0;

        for entry in entries.into_iter().take(MAX_BATCH_ENTRIES) {
            let Some(entry) = sanitize_entry(entry) else {
                continue;
            };
            if buffer.len() >= MAX_LOGS_PER_NODE {
                buffer.pop_front();
            }
            buffer.push_back(entry);
            accepted += 1;
        }

        accepted
    }

    /// 返回某节点最近的若干条日志,按发生时间升序保留。
    pub async fn list(&self, node_id: &str, limit: usize) -> Vec<AgentLogEntry> {
        let guard = self.inner.read().await;
        let Some(buffer) = guard.get(node_id) else {
            return Vec::new();
        };

        let limit = limit.clamp(1, MAX_LOGS_PER_NODE);
        let start = buffer.len().saturating_sub(limit);
        buffer.iter().skip(start).cloned().collect()
    }

    /// 清理已经不在注册表中的节点日志,避免长期运行时缓冲只增不减。
    pub async fn forget_missing(&self, live_node_ids: &[String]) -> usize {
        let live: HashSet<&str> = live_node_ids.iter().map(String::as_str).collect();
        let mut guard = self.inner.write().await;
        let before = guard.len();
        guard.retain(|node_id, _| live.contains(node_id.as_str()));
        before.saturating_sub(guard.len())
    }
}

fn sanitize_entry(mut entry: AgentLogEntry) -> Option<AgentLogEntry> {
    let message = entry.message.trim();
    if message.is_empty() {
        return None;
    }

    entry.message = truncate_to_byte_boundary(message, MAX_LOG_MESSAGE_BYTES).to_string();
    if DateTime::parse_from_rfc3339(&entry.occurred_at).is_err() {
        entry.occurred_at = Utc::now().to_rfc3339();
    }
    Some(entry)
}

fn truncate_to_byte_boundary(value: &str, max_bytes: usize) -> &str {
    if value.len() <= max_bytes {
        return value;
    }

    let mut end = max_bytes;
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    &value[..end]
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use ximonitor_proto::NoticeLevel;

    use super::{AgentLogEntry, AgentLogStore, MAX_LOGS_PER_NODE, truncate_to_byte_boundary};

    #[tokio::test]
    async fn record_entries_caps_per_node_and_sanitizes_payloads() {
        let store = AgentLogStore::new();
        let entries = (0..(MAX_LOGS_PER_NODE + 10))
            .map(|index| AgentLogEntry {
                occurred_at: "invalid".to_string(),
                level: NoticeLevel::Info,
                message: format!("entry-{index}"),
            })
            .collect();

        let accepted = store.record_entries("hk-01", entries).await;
        assert_eq!(accepted, 64);

        let logs = store.list("hk-01", MAX_LOGS_PER_NODE).await;
        assert_eq!(logs.len(), 64);
        assert!(logs.iter().all(|entry| !entry.message.is_empty()));
        assert!(
            logs.iter()
                .all(|entry| chrono::DateTime::parse_from_rfc3339(&entry.occurred_at).is_ok())
        );
    }

    #[tokio::test]
    async fn forget_missing_prunes_retired_node_buffers() {
        let store = AgentLogStore::new();
        let entry = AgentLogEntry {
            occurred_at: Utc::now().to_rfc3339(),
            level: NoticeLevel::Warn,
            message: "reconnecting".to_string(),
        };
        store.record_entries("hk-01", vec![entry.clone()]).await;
        store.record_entries("jp-01", vec![entry]).await;

        let removed = store.forget_missing(&["jp-01".to_string()]).await;
        assert_eq!(removed, 1);
        assert!(store.list("hk-01", 10).await.is_empty());
        assert_eq!(store.list("jp-01", 10).await.len(), 1);
    }

    #[test]
    fn truncate_to_byte_boundary_preserves_utf8() {
        let value = "日志-abcdef";
        let truncated = truncate_to_byte_boundary(value, 5);
        assert!(truncated.is_char_boundary(truncated.len()));
        assert_eq!(truncated, "日");
    }
}
