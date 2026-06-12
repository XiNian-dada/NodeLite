use super::*;

use nodelite_proto::BrowserMessage;

/// 未认证的浏览器 WebSocket 升级握手必须被拒为 HTTP 401。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn browser_ws_rejects_unauthenticated_connection() -> Result<()> {
    let server = TestServer::start().await?;
    TestBrowserClient::expect_unauthorized(&server).await?;
    server.shutdown().await
}

/// 连接建立后,服务端立即下发一条全量 `InitialState`。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn browser_ws_sends_initial_state_on_connect() -> Result<()> {
    let server = TestServer::start().await?;
    let mut browser = TestBrowserClient::connect(&server).await?;

    let message = browser.next_message(TEST_TIMEOUT).await?;
    match message {
        BrowserMessage::InitialState {
            overview, nodes, ..
        } => {
            assert_eq!(overview.total_nodes, 0);
            assert!(nodes.is_empty());
        }
        other => panic!("expected InitialState, got {other:?}"),
    }

    browser.close().await?;
    server.shutdown().await
}

/// 浏览器连接后再有 agent 注册并上报,浏览器应收到该节点的增量 `NodeUpsert`。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn browser_ws_pushes_node_upsert_when_agent_registers() -> Result<()> {
    let server = TestServer::start().await?;
    // 先连浏览器,确保它的 InitialState 是空的,后续节点变化只能通过增量到达。
    let mut browser = TestBrowserClient::connect(&server).await?;
    let initial = browser.next_message(TEST_TIMEOUT).await?;
    assert!(matches!(initial, BrowserMessage::InitialState { .. }));

    // agent 注册 + 上报 → 触发 SharedState 脏信号。
    let node = server
        .issue_node("itest-browser-01", "Integration Browser 01")
        .await?;
    let mut agent = TestAgent::connect(&server, &node).await?;
    agent.send_fake_metrics(1).await?;

    // 浏览器应收到该节点的 NodeUpsert(跳过其间可能的 OverviewUpdate)。
    let upsert = browser
        .next_matching(TEST_TIMEOUT, |message| {
            matches!(
                message,
                BrowserMessage::NodeUpsert { node, .. } if node.identity.node_id == "itest-browser-01"
            )
        })
        .await?;
    match upsert {
        BrowserMessage::NodeUpsert { node, .. } => {
            assert_eq!(node.identity.node_id, "itest-browser-01");
            assert!(node.online);
        }
        other => panic!("expected NodeUpsert, got {other:?}"),
    }

    agent.disconnect().await?;
    browser.close().await?;
    server.shutdown().await
}

/// 节点内容变化后,浏览器应收到同一节点的第二条 `NodeUpsert`(增量更新,而非只推一次)。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn browser_ws_pushes_second_upsert_when_node_changes() -> Result<()> {
    let server = TestServer::start().await?;
    let mut browser = TestBrowserClient::connect(&server).await?;
    let initial = browser.next_message(TEST_TIMEOUT).await?;
    assert!(matches!(initial, BrowserMessage::InitialState { .. }));

    let node = server
        .issue_node("itest-browser-change", "Integration Browser Change")
        .await?;
    let mut agent = TestAgent::connect(&server, &node).await?;

    // 第一次上报 → 第一条 NodeUpsert。先等到它,确保 uptime=2 的上报落在下一个 diff 周期。
    // fake_snapshot(n) 的 cpu_usage_percent = 12.5 + n%7,可精确区分两次上报。
    agent.send_fake_metrics(1).await?;
    browser
        .next_matching(TEST_TIMEOUT, |message| {
            matches!(
                message,
                BrowserMessage::NodeUpsert { node, .. }
                    if node.identity.node_id == "itest-browser-change"
                        && node.snapshot.as_ref().is_some_and(|s| s.cpu_usage_percent == Some(13.5))
            )
        })
        .await?;

    // 内容变化的第二次上报 → 必须再次收到该节点的 NodeUpsert,携带新快照。
    agent.send_fake_metrics(2).await?;
    let upsert = browser
        .next_matching(TEST_TIMEOUT, |message| {
            matches!(
                message,
                BrowserMessage::NodeUpsert { node, .. }
                    if node.identity.node_id == "itest-browser-change"
            )
        })
        .await?;
    match upsert {
        BrowserMessage::NodeUpsert { node, .. } => {
            assert_eq!(
                node.snapshot.as_ref().and_then(|s| s.cpu_usage_percent),
                Some(14.5),
                "second upsert should carry the updated snapshot"
            );
        }
        other => panic!("expected NodeUpsert, got {other:?}"),
    }

    agent.disconnect().await?;
    browser.close().await?;
    server.shutdown().await
}

/// agent 断开后,浏览器应收到该节点 `online == false` 的增量 `NodeUpsert`。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn browser_ws_pushes_offline_upsert_when_agent_disconnects() -> Result<()> {
    let server = TestServer::start().await?;
    let mut browser = TestBrowserClient::connect(&server).await?;
    let initial = browser.next_message(TEST_TIMEOUT).await?;
    assert!(matches!(initial, BrowserMessage::InitialState { .. }));

    let node = server
        .issue_node("itest-browser-offline", "Integration Browser Offline")
        .await?;
    let mut agent = TestAgent::connect(&server, &node).await?;
    agent.send_fake_metrics(1).await?;
    browser
        .next_matching(TEST_TIMEOUT, |message| {
            matches!(
                message,
                BrowserMessage::NodeUpsert { node, .. }
                    if node.identity.node_id == "itest-browser-offline" && node.online
            )
        })
        .await?;

    agent.disconnect().await?;
    server
        .wait_for_node_offline("itest-browser-offline", LIVE_REFRESH_TIMEOUT)
        .await?;

    let offline = browser
        .next_matching(LIVE_REFRESH_TIMEOUT, |message| {
            matches!(
                message,
                BrowserMessage::NodeUpsert { node, .. }
                    if node.identity.node_id == "itest-browser-offline" && !node.online
            )
        })
        .await?;
    assert!(matches!(offline, BrowserMessage::NodeUpsert { .. }));

    browser.close().await?;
    server.shutdown().await
}

/// 没有任何节点变化时,去抖 tick 不应产生消息:InitialState 之后保持静默。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn browser_ws_stays_silent_without_node_changes() -> Result<()> {
    let server = TestServer::start().await?;
    let mut browser = TestBrowserClient::connect(&server).await?;
    let initial = browser.next_message(TEST_TIMEOUT).await?;
    assert!(matches!(initial, BrowserMessage::InitialState { .. }));

    // 覆盖至少两个去抖周期(1s/次):dirty=false 的 tick 不得发任何增量或概览。
    browser
        .expect_no_message(std::time::Duration::from_millis(2500))
        .await?;

    browser.close().await?;
    server.shutdown().await
}

/// 客户端发送应用层 `Ping`,服务端必须回 `Pong`。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn browser_ws_replies_pong_to_ping() -> Result<()> {
    let server = TestServer::start().await?;
    let mut browser = TestBrowserClient::connect(&server).await?;
    let initial = browser.next_message(TEST_TIMEOUT).await?;
    assert!(matches!(initial, BrowserMessage::InitialState { .. }));

    browser.send_ping().await?;
    let pong = browser
        .next_matching(TEST_TIMEOUT, |message| {
            matches!(message, BrowserMessage::Pong)
        })
        .await?;
    assert!(matches!(pong, BrowserMessage::Pong));

    browser.close().await?;
    server.shutdown().await
}
