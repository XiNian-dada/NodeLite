//! 集中 diff 任务:把节点视图变化推导为增量消息并广播给所有浏览器会话。

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use anyhow::Context;
use axum::extract::ws::Utf8Bytes;
use nodelite_proto::{BrowserMessage, NodeListItem};
use tokio::sync::{Mutex, broadcast};
use tokio_util::sync::CancellationToken;

use super::SharedState;

/// 集中 diff 任务一个 tick 的全部增量。整批只占一个通道槽位,节点数量再大也不会
/// 单独把订阅者挤到 `Lagged`;帧只序列化一次,所有会话共享同一份字节。
#[derive(Clone)]
pub(crate) struct BrowserIncrementalUpdate {
    pub(crate) revision: u64,
    pub(crate) frames: Arc<[Utf8Bytes]>,
}

/// 一个槽位现在可能持有全部节点的增量,而慢会话会让已发送的批次继续驻留,
/// 所以容量决定内存上限(容量 × 节点数)。16 个 tick 约等于落后 16 秒,
/// 这样的会话走一次全量重同步比为它保留更多批次更划算。
pub(super) const BROWSER_INCREMENTAL_CHANNEL_CAPACITY: usize = 16;

/// 全量帧的复用时限。revision 在高频上报下每几毫秒就变一次,不能作为复用条件,
/// 否则同一 tick 内落后的会话仍会各自重建。
const BROWSER_INITIAL_STATE_MAX_AGE: Duration = Duration::from_secs(1);

struct CachedInitialState {
    revision: u64,
    built_at: Instant,
    frame: Utf8Bytes,
}

/// 浏览器会话共享的单份全量帧。
#[derive(Default)]
pub(super) struct BrowserResyncCache {
    slot: Mutex<Option<CachedInitialState>>,
    published_revision: AtomicU64,
    builds: AtomicU64,
}

impl BrowserResyncCache {
    fn advance_published_revision(&self, revision: u64) {
        self.published_revision
            .fetch_max(revision, Ordering::AcqRel);
    }
}

impl SharedState {
    /// 返回全量 `InitialState` 帧及其 revision baseline。调用方必须已持有增量订阅:
    /// 只有快照不早于最后一批已发布增量时,快照加上后续批次才能收敛到最新状态,
    /// 而之后发布的批次订阅者都能收到。因此每个 tick 最多构建一次全量。
    pub(crate) async fn browser_initial_state_frame(&self) -> anyhow::Result<(u64, Utf8Bytes)> {
        let cache = &self.browser_resync;
        let mut slot = cache.slot.lock().await;
        let published = cache.published_revision.load(Ordering::Acquire);
        if let Some(cached) = slot.as_ref()
            && cached.revision >= published
            && cached.built_at.elapsed() <= BROWSER_INITIAL_STATE_MAX_AGE
        {
            return Ok((cached.revision, cached.frame.clone()));
        }

        cache.builds.fetch_add(1, Ordering::Relaxed);
        let snapshot = self.browser_snapshot().await;
        let revision = snapshot.revision;
        let frame = browser_frame(&BrowserMessage::InitialState {
            generated_at: snapshot.generated_at,
            overview: snapshot.overview,
            nodes: snapshot.nodes,
        })?;
        *slot = Some(CachedInitialState {
            revision,
            built_at: Instant::now(),
            frame: frame.clone(),
        });
        Ok((revision, frame))
    }

    #[cfg(test)]
    pub(super) fn browser_initial_state_build_count(&self) -> u64 {
        self.browser_resync.builds.load(Ordering::Relaxed)
    }
}

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
                run_tick(&shared, &mut last_nodes, &mut dirty).await;
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

pub(super) fn browser_frame(message: &BrowserMessage) -> anyhow::Result<Utf8Bytes> {
    serde_json::to_string(message)
        .map(Utf8Bytes::from)
        .context("failed to serialize browser message")
}

pub(super) async fn run_tick(
    shared: &SharedState,
    last_nodes: &mut HashMap<String, NodeListItem>,
    dirty: &mut bool,
) {
    // 必须先于 receiver_count 推进:本 tick 会消费掉脏标记,此后才订阅的会话
    // 等不到下一批增量,只能靠这里让它拒绝旧缓存、取到包含这些变化的快照。
    shared
        .browser_resync
        .advance_published_revision(shared.nodes_revision.load(Ordering::Acquire));
    if shared.browser_incremental_tx.receiver_count() == 0 {
        *shared.browser_resync.slot.lock().await = None;
        // 不能保留旧基线:节点在无人订阅期间变化后又变回基线值时,diff 判定为未变,
        // 新会话手里的中间值就再也得不到修正。清空后首批增量等于全量,结果总是收敛。
        *last_nodes = HashMap::new();
        *dirty = false;
        return;
    }
    if !*dirty {
        return;
    }
    match broadcast_incremental_updates(shared, last_nodes).await {
        Ok(()) => *dirty = false,
        Err(error) => {
            // 基线未推进且保持 dirty,下个 tick 会重新计算这批变化。
            tracing::warn!(error = %error, "browser incremental task failed to broadcast updates");
        }
    }
}

async fn broadcast_incremental_updates(
    shared: &SharedState,
    last_nodes: &mut HashMap<String, NodeListItem>,
) -> anyhow::Result<()> {
    let snapshot = shared.browser_snapshot().await;
    let generated_at = snapshot.generated_at;
    let revision = snapshot.revision;
    let current = snapshot.nodes;

    let diff = diff_node_lists(last_nodes, &current);
    let mut frames = Vec::with_capacity(diff.upserts.len() + diff.removed.len() + 1);
    for node in &diff.upserts {
        frames.push(browser_frame(&BrowserMessage::NodeUpsert {
            generated_at,
            node: Box::new((*node).clone()),
        })?);
    }
    for node_id in diff.removed {
        frames.push(browser_frame(&BrowserMessage::NodeRemoved {
            generated_at,
            node_id,
        })?);
    }
    frames.push(browser_frame(&BrowserMessage::OverviewUpdate {
        generated_at,
        overview: snapshot.overview,
    })?);

    *last_nodes = index_by_node_id(current);

    shared.browser_resync.advance_published_revision(revision);
    let _ = shared
        .browser_incremental_tx
        .send(BrowserIncrementalUpdate {
            revision,
            frames: frames.into(),
        });
    Ok(())
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
