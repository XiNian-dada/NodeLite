//! 集中 diff 任务:把节点视图变化推导为增量消息并广播给所有浏览器会话。

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use nodelite_proto::{BrowserMessage, NodeListItem};
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;

use super::SharedState;

/// 集中 diff 任务广播给浏览器会话的增量消息。
#[derive(Clone)]
pub(crate) struct BrowserIncrementalUpdate {
    pub(crate) revision: u64,
    pub(crate) message: Arc<BrowserMessage>,
}

/// 浏览器增量消息广播通道容量。集中 diff 任务每秒最多广播 ~400 条消息(200 节点
/// 全变 × 每节点 1 upsert + 1 overview),慢连接若落后超过 256 条会收到 Lagged,
/// 届时重发 InitialState 全量同步。
pub(super) const BROWSER_INCREMENTAL_CHANNEL_CAPACITY: usize = 256;

/// 启动集中 diff 后台任务,订阅脏信号、去抖并广播增量消息给所有浏览器会话。
/// 返回 JoinHandle 供调用者纳入 shutdown 生命周期。
pub(crate) fn spawn_browser_incremental_task(
    shared: SharedState,
    shutdown: CancellationToken,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        run_browser_incremental_task(shared, shutdown).await;
    })
}

async fn run_browser_incremental_task(shared: SharedState, shutdown: CancellationToken) {
    use tokio::time::{MissedTickBehavior, interval};

    let mut updates = shared.subscribe_browser_updates();
    let mut debounce = interval(Duration::from_secs(1));
    debounce.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut dirty = false;
    let mut last_nodes: HashMap<String, NodeListItem> = HashMap::new();

    loop {
        tokio::select! {
            _ = shutdown.cancelled() => return,
            recv = updates.recv() => {
                match recv {
                    Ok(_) => dirty = true,
                    Err(broadcast::error::RecvError::Lagged(_)) => {
                        // 落后丢信号:强制下次 tick 重算并广播完整增量
                        dirty = true;
                    }
                    Err(broadcast::error::RecvError::Closed) => return,
                }
            }
            _ = debounce.tick() => {
                if dirty {
                    dirty = false;
                    broadcast_incremental_updates(&shared, &mut last_nodes).await;
                }
            }
        }
    }
}

/// 一次重算相对上次发送快照的增量:新增/变更的行 + 消失的行 id。
struct NodeListDiff<'a> {
    upserts: Vec<&'a NodeListItem>,
    removed: Vec<String>,
}

fn diff_node_lists<'a>(
    last_nodes: &HashMap<String, NodeListItem>,
    current: &'a [NodeListItem],
) -> NodeListDiff<'a> {
    let mut seen: HashSet<&str> = HashSet::with_capacity(current.len());
    let mut upserts = Vec::new();
    for node in current {
        seen.insert(node.identity.node_id.as_str());
        if last_nodes.get(&node.identity.node_id) != Some(node) {
            upserts.push(node);
        }
    }
    let removed = last_nodes
        .keys()
        .filter(|id| !seen.contains(id.as_str()))
        .cloned()
        .collect();
    NodeListDiff { upserts, removed }
}

fn index_by_node_id(nodes: Vec<NodeListItem>) -> HashMap<String, NodeListItem> {
    nodes
        .into_iter()
        .map(|node| (node.identity.node_id.clone(), node))
        .collect()
}

async fn broadcast_incremental_updates(
    shared: &SharedState,
    last_nodes: &mut HashMap<String, NodeListItem>,
) {
    let snapshot = shared.browser_snapshot().await;
    let generated_at = snapshot.generated_at;
    let revision = snapshot.revision;
    let current = snapshot.nodes;

    let diff = diff_node_lists(last_nodes, &current);
    let mut messages: Vec<BrowserMessage> =
        Vec::with_capacity(diff.upserts.len() + diff.removed.len() + 1);
    messages.extend(diff.upserts.iter().map(|node| BrowserMessage::NodeUpsert {
        generated_at,
        node: Box::new((*node).clone()),
    }));
    messages.extend(
        diff.removed
            .into_iter()
            .map(|node_id| BrowserMessage::NodeRemoved {
                generated_at,
                node_id,
            }),
    );
    messages.push(BrowserMessage::OverviewUpdate {
        generated_at,
        overview: snapshot.overview,
    });

    *last_nodes = index_by_node_id(current);

    for message in messages {
        let _ = shared
            .browser_incremental_tx
            .send(BrowserIncrementalUpdate {
                revision,
                message: Arc::new(message),
            });
    }
}

#[cfg(test)]
mod tests {
    use nodelite_proto::NodeListIdentity;

    use super::*;

    fn list_item(node_id: &str) -> NodeListItem {
        NodeListItem {
            identity: NodeListIdentity {
                node_id: node_id.to_string(),
                node_label: format!("{node_id} label"),
                hostname: format!("{node_id}.internal"),
                tags: Vec::new(),
            },
            geoip_country: None,
            geoip_city: None,
            geoip_latitude: None,
            geoip_longitude: None,
            location_override_country: None,
            location_override_city: None,
            location_override_latitude: None,
            location_override_longitude: None,
            snapshot: None,
            latency_ms: None,
            online: true,
        }
    }

    fn upsert_ids<'a>(diff: &NodeListDiff<'a>) -> Vec<&'a str> {
        diff.upserts
            .iter()
            .map(|node| node.identity.node_id.as_str())
            .collect()
    }

    #[test]
    fn diff_reports_new_changed_and_missing_nodes() {
        let last = index_by_node_id(vec![
            list_item("unchanged"),
            list_item("changed"),
            list_item("removed"),
        ]);
        let mut changed = list_item("changed");
        changed.online = false;
        let current = vec![list_item("unchanged"), changed, list_item("added")];

        let diff = diff_node_lists(&last, &current);

        assert_eq!(upsert_ids(&diff), vec!["changed", "added"]);
        assert_eq!(diff.removed, vec!["removed".to_string()]);
    }

    #[test]
    fn diff_of_unchanged_or_empty_lists_is_empty() {
        let last = index_by_node_id(vec![list_item("hk-01")]);
        let current = [list_item("hk-01")];
        let unchanged = diff_node_lists(&last, &current);
        assert!(unchanged.upserts.is_empty() && unchanged.removed.is_empty());

        let empty = diff_node_lists(&HashMap::new(), &[]);
        assert!(empty.upserts.is_empty() && empty.removed.is_empty());
    }

    #[test]
    fn index_by_node_id_keys_each_item_by_its_id() {
        let indexed = index_by_node_id(vec![list_item("hk-01"), list_item("jp-01")]);

        assert_eq!(indexed.len(), 2);
        assert_eq!(indexed["hk-01"].identity.node_id, "hk-01");
    }
}
