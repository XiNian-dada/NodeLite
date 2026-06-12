//! 浏览器 WebSocket 会话(`/ws/browser`)。
//!
//! 与 agent `/ws` 区分:浏览器通道是**只读监控推送**,认证由 `require_readonly_auth`
//! 中间件在升级前完成(Basic Auth + 可选 2FA)。会话流程:
//!   1. 准入:每 IP 并发上限(RAII permit,与 agent 连接各自计数);
//!   2. 升级为 WebSocket;
//!   3. 立即下发全量 `InitialState`;
//!   4. 订阅 `SharedState` 脏信号,去抖(≤1/秒)后重算节点列表、与上次发送的
//!      快照做 diff,发出 `NodeUpsert` / `NodeRemoved` 增量 + `OverviewUpdate`;
//!   5. 处理客户端应用层 `Ping`(回 `Pong`);
//!   6. 连接关闭时 RAII permit 自动归还配额、广播订阅自动退订。
//!
//! 增量(而非全量)是带宽收益的来源:单节点变化只发该节点一行。服务端用
//! "重算 + diff" 推导出变化行 —— 既无需注册表暴露细粒度变更钩子,也天然覆盖
//! 节点移除(diff 发现某 id 消失即发 `NodeRemoved`)。

use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::time::Duration;

use anyhow::anyhow;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{ConnectInfo, State};
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Response};
use chrono::Utc;
use futures::stream::SplitSink;
use futures::{SinkExt, StreamExt};
use nodelite_proto::{BrowserMessage, NodeListItem};
use tokio::sync::broadcast;
use tokio::time::{MissedTickBehavior, interval};
use tracing::warn;

use crate::AppState;
use crate::admission::{resolve_client_ip, ws_admission_error_response};
use crate::state::SharedState;

type BrowserSink = SplitSink<WebSocket, Message>;

/// 浏览器视图增量去抖间隔:脏信号到达后最多每秒重算一次并发出增量。
const BROWSER_PUSH_DEBOUNCE: Duration = Duration::from_secs(1);

/// `/ws/browser` 入口。认证已由 `require_readonly_auth` 中间件完成;这里只做
/// 并发准入(每 IP 上限)与协议升级。
pub async fn ws_browser_handler(
    State(state): State<AppState>,
    ConnectInfo(peer_addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Response {
    let config = state.shared.config();
    let client_ip = resolve_client_ip(&config.trusted_proxies, peer_addr, &headers);
    let permit = match state.browser_ws_admission.try_acquire(client_ip) {
        Ok(permit) => permit,
        Err(error) => return ws_admission_error_response(error),
    };
    let max_message_bytes = config.max_message_bytes;
    ws.max_frame_size(max_message_bytes)
        .max_message_size(max_message_bytes)
        .on_upgrade(move |socket| async move {
            // permit 持有到会话结束;drop 时自动把连接配额归还给该 IP。
            let _permit = permit;
            if let Err(error) = run_browser_session(state.shared.clone(), socket).await {
                warn!(error = %error, "browser websocket session ended with error");
            }
        })
        .into_response()
}

/// 单个浏览器会话主循环。
async fn run_browser_session(shared: SharedState, socket: WebSocket) -> anyhow::Result<()> {
    let (mut sender, mut receiver) = socket.split();
    let mut updates = shared.subscribe_browser_updates();

    // 1. 立即发送全量 InitialState,并记录"上次发送的快照"用于后续 diff。
    let mut last_nodes = send_initial_state(&shared, &mut sender).await?;

    // 2. 去抖计时器:脏信号只置 `dirty`,真正的重算 + 发送推迟到下一次 tick。
    //    `interval` 首个 tick 立即触发,此时 dirty=false 是无害空转。
    let mut debounce = interval(BROWSER_PUSH_DEBOUNCE);
    debounce.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut dirty = false;

    loop {
        tokio::select! {
            biased;
            incoming = receiver.next() => {
                match incoming {
                    None | Some(Err(_)) => return Ok(()), // 客户端关闭或传输错误 → 结束会话
                    Some(Ok(message)) => {
                        if handle_client_message(&mut sender, message).await? {
                            return Ok(());
                        }
                    }
                }
            }
            recv = updates.recv() => {
                match recv {
                    Ok(_) => dirty = true,
                    Err(broadcast::error::RecvError::Lagged(_)) => {
                        // 落后丢信号:客户端状态可能不一致 → 重发全量 InitialState 强制重同步。
                        last_nodes = send_initial_state(&shared, &mut sender).await?;
                        dirty = false;
                    }
                    Err(broadcast::error::RecvError::Closed) => return Ok(()),
                }
            }
            _ = debounce.tick() => {
                if dirty {
                    dirty = false;
                    push_incremental_updates(&shared, &mut sender, &mut last_nodes).await?;
                }
            }
        }
    }
}

/// 客户端入站消息的处理决策,由 [`classify_client_message`] 纯函数推导。
#[derive(Debug, PartialEq, Eq)]
enum ClientAction {
    /// 应用层 `Ping` → 回 `Pong`。
    ReplyPong,
    /// 客户端发起关闭 → 结束会话。
    End,
    /// 其它消息一律忽略(协议向前兼容;协议级 Ping/Pong 帧由底层自动应答)。
    Ignore,
}

fn classify_client_message(message: &Message) -> ClientAction {
    match message {
        Message::Text(text) => {
            if matches!(
                serde_json::from_str::<BrowserMessage>(text.as_str()),
                Ok(BrowserMessage::Ping)
            ) {
                ClientAction::ReplyPong
            } else {
                ClientAction::Ignore
            }
        }
        Message::Close(_) => ClientAction::End,
        _ => ClientAction::Ignore,
    }
}

/// 处理客户端发来的消息。返回 `true` 表示应结束会话。
async fn handle_client_message(sender: &mut BrowserSink, message: Message) -> anyhow::Result<bool> {
    match classify_client_message(&message) {
        ClientAction::ReplyPong => {
            send_browser_message(sender, &BrowserMessage::Pong).await?;
            Ok(false)
        }
        ClientAction::End => Ok(true),
        ClientAction::Ignore => Ok(false),
    }
}

/// 发送全量 `InitialState`,返回按 `node_id` 索引的"已发送快照",供后续 diff。
async fn send_initial_state(
    shared: &SharedState,
    sender: &mut BrowserSink,
) -> anyhow::Result<HashMap<String, NodeListItem>> {
    let nodes = shared.list_node_summaries().await;
    let overview = shared.overview_snapshot().await;
    let message = BrowserMessage::InitialState {
        generated_at: Utc::now(),
        overview,
        nodes: nodes.clone(),
    };
    send_browser_message(sender, &message).await?;
    Ok(index_by_node_id(nodes))
}

/// 一次重算相对上次发送快照的增量:新增/变更的行 + 消失的行 id。
struct NodeListDiff<'a> {
    upserts: Vec<&'a NodeListItem>,
    removed: Vec<String>,
}

/// 与上次发送的快照逐行对比:内容不同(或新出现)的行进 `upserts`,
/// 上次有、这次没有的 id 进 `removed`。行内容未变时不产生任何输出。
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

/// 重算当前节点列表,与上次发送的快照 diff,发出增量 + 概览更新。
async fn push_incremental_updates(
    shared: &SharedState,
    sender: &mut BrowserSink,
    last_nodes: &mut HashMap<String, NodeListItem>,
) -> anyhow::Result<()> {
    let current = shared.list_node_summaries().await;
    let generated_at = Utc::now();

    let NodeListDiff { upserts, removed } = diff_node_lists(last_nodes, &current);
    for node in upserts {
        send_browser_message(
            sender,
            &BrowserMessage::NodeUpsert {
                generated_at,
                node: Box::new(node.clone()),
            },
        )
        .await?;
    }
    for node_id in removed {
        send_browser_message(
            sender,
            &BrowserMessage::NodeRemoved {
                generated_at,
                node_id,
            },
        )
        .await?;
    }

    *last_nodes = index_by_node_id(current);

    // 概览聚合每次脏 tick 重算一次(已被去抖到 ≤1/秒),惰性计算避免高频重算。
    let overview = shared.overview_snapshot().await;
    send_browser_message(
        sender,
        &BrowserMessage::OverviewUpdate {
            generated_at,
            overview,
        },
    )
    .await
}

fn index_by_node_id(nodes: Vec<NodeListItem>) -> HashMap<String, NodeListItem> {
    nodes
        .into_iter()
        .map(|node| (node.identity.node_id.clone(), node))
        .collect()
}

async fn send_browser_message(
    sender: &mut BrowserSink,
    message: &BrowserMessage,
) -> anyhow::Result<()> {
    let payload = serde_json::to_string(message)
        .map_err(|error| anyhow!("failed to serialize browser message: {error}"))?;
    sender
        .send(Message::Text(payload.into()))
        .await
        .map_err(|error| anyhow!("failed to send browser message: {error}"))
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
    fn diff_reports_new_node_as_upsert() {
        let last = HashMap::new();
        let current = vec![list_item("hk-01")];

        let diff = diff_node_lists(&last, &current);

        assert_eq!(upsert_ids(&diff), vec!["hk-01"]);
        assert!(diff.removed.is_empty());
    }

    #[test]
    fn diff_reports_changed_node_as_upsert() {
        let last = index_by_node_id(vec![list_item("hk-01")]);
        let mut changed = list_item("hk-01");
        changed.latency_ms = Some(42);
        let current = vec![changed];

        let diff = diff_node_lists(&last, &current);

        assert_eq!(upsert_ids(&diff), vec!["hk-01"]);
        assert!(diff.removed.is_empty());
    }

    #[test]
    fn diff_skips_unchanged_node() {
        let last = index_by_node_id(vec![list_item("hk-01")]);
        let current = vec![list_item("hk-01")];

        let diff = diff_node_lists(&last, &current);

        assert!(diff.upserts.is_empty());
        assert!(diff.removed.is_empty());
    }

    #[test]
    fn diff_reports_missing_node_as_removed() {
        let last = index_by_node_id(vec![list_item("hk-01"), list_item("jp-01")]);
        let current = vec![list_item("hk-01")];

        let diff = diff_node_lists(&last, &current);

        assert!(diff.upserts.is_empty());
        assert_eq!(diff.removed, vec!["jp-01".to_string()]);
    }

    #[test]
    fn diff_handles_mixed_changes_in_one_pass() {
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
    fn diff_of_two_empty_lists_is_empty() {
        let diff = diff_node_lists(&HashMap::new(), &[]);

        assert!(diff.upserts.is_empty());
        assert!(diff.removed.is_empty());
    }

    #[test]
    fn index_by_node_id_keys_each_item_by_its_id() {
        let indexed = index_by_node_id(vec![list_item("hk-01"), list_item("jp-01")]);

        assert_eq!(indexed.len(), 2);
        assert_eq!(indexed["hk-01"].identity.node_id, "hk-01");
        assert_eq!(indexed["jp-01"].identity.node_id, "jp-01");
    }

    #[test]
    fn classify_replies_pong_to_app_level_ping() {
        let payload = serde_json::to_string(&BrowserMessage::Ping).expect("ping should serialize");

        let action = classify_client_message(&Message::Text(payload.into()));

        assert_eq!(action, ClientAction::ReplyPong);
    }

    #[test]
    fn classify_ignores_other_browser_messages() {
        let payload = serde_json::to_string(&BrowserMessage::Pong).expect("pong should serialize");

        let action = classify_client_message(&Message::Text(payload.into()));

        assert_eq!(action, ClientAction::Ignore);
    }

    #[test]
    fn classify_ignores_invalid_json_text() {
        let action = classify_client_message(&Message::Text("{not-json}".into()));

        assert_eq!(action, ClientAction::Ignore);
    }

    #[test]
    fn classify_ignores_binary_frames() {
        let action = classify_client_message(&Message::Binary(vec![1, 2, 3].into()));

        assert_eq!(action, ClientAction::Ignore);
    }

    #[test]
    fn classify_ends_session_on_close_frame() {
        let action = classify_client_message(&Message::Close(None));

        assert_eq!(action, ClientAction::End);
    }
}
