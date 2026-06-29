use std::net::SocketAddr;
use std::time::Duration;

use anyhow::{Result, anyhow};
use futures::{SinkExt, StreamExt};
use nodelite_agent::collector::new_collector;
use nodelite_agent::session::{AgentLogBuffer, run_forever};
use nodelite_proto::{AgentConfig, NodeIdentity, NoticeLevel, ServerNoticeMessage, WireMessage};
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::accept_async;
use tokio_tungstenite::tungstenite::Message;

mod common;
use common::TempDir;

/// 指向给定地址的 Agent 配置。`connect_timeout_secs` 取 2s,`report_interval_secs`
/// 取 5s,确保测试窗口内不会因为指标上报而产生额外流量。
fn test_config(local_addr: SocketAddr) -> AgentConfig {
    AgentConfig {
        node_id: "reconnect-node-01".to_string(),
        node_label: "Reconnect Node 01".to_string(),
        server: format!("ws://{local_addr}/ws"),
        token: "reconnect-token".to_string(),
        connect_timeout_secs: 2,
        report_interval_secs: 5,
        max_incoming_message_bytes: 65536,
        insecure_transport_warn_interval_secs: 900,
        tags: vec![],
        hostname_override: None,
    }
}

fn test_identity(config: &AgentConfig) -> NodeIdentity {
    NodeIdentity {
        node_id: config.node_id.clone(),
        node_label: config.node_label.clone(),
        hostname: "localhost".to_string(),
        os: "test".to_string(),
        kernel_version: None,
        cpu_model: None,
        cpu_cores: 1,
        agent_version: "0.1.0-test".to_string(),
        boot_time: None,
        tags: vec![],
    }
}

/// 验证认证前断连后的首次退避确实落在 `reconnect_delay(0)` 的 [1s, 5s] 窗口内:
/// 推进不足 1s 不得重连;推进越过 5s 必须重连。
#[tokio::test]
async fn test_agent_reconnect_backoff_with_mock_time() -> Result<()> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let local_addr = listener.local_addr()?;
    let temp_dir = TempDir::new("nodelite-agent-reconnect-test");
    let config_path = temp_dir.path().join("agent.toml");

    let config = test_config(local_addr);
    let identity = test_identity(&config);
    let collector = new_collector();
    let log_buffer = AgentLogBuffer::default();

    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
    let agent_task = tokio::spawn(run_forever(
        config,
        collector,
        identity,
        config_path,
        log_buffer,
        async move {
            let _ = shutdown_rx.await;
        },
    ));

    // 第一次连接:完成 WebSocket 握手、读取 Hello,然后在认证前主动断开,迫使
    // Agent 进入常规重连退避。这里必须先走真实 I/O,再切到虚拟时间,否则
    // `pause()` 会和 connect timeout / WebSocket 握手抢跑,导致首连本身变得不稳定。
    let (stream1, _) = tokio::select! {
        res = listener.accept() => res?,
        _ = tokio::time::sleep(Duration::from_secs(5)) => {
            panic!("timed out waiting for first connection");
        }
    };
    serve_pre_auth_disconnect(stream1).await?;

    tokio::time::pause();
    tokio::task::yield_now().await;

    // 退避下限:首次退避至少 1s,推进 0.9s 绝不应触发重连。
    let mut accept_fut = Box::pin(listener.accept());
    tokio::time::advance(Duration::from_millis(800)).await;
    assert!(
        futures::poll!(accept_fut.as_mut()).is_pending(),
        "agent reconnected before the 1s backoff floor elapsed",
    );

    // 退避上限:首次退避至多 5s,推进越过 5s 后必须发生第二次连接。
    tokio::time::advance(Duration::from_secs(6)).await;
    let (stream2, _) = accept_fut.await?;
    drop(stream2);

    let _ = shutdown_tx.send(());
    agent_task
        .await
        .expect("agent reconnect task should not panic")?;
    Ok(())
}

/// 验证 token 过期走的是独立的长退避路径(首次 30s),而非常规的 1–5s:
/// 在常规退避早已到期的 6s 处不得重连,推进越过 30s 后才重连。
#[tokio::test]
async fn test_agent_token_expired_uses_long_backoff() -> Result<()> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let local_addr = listener.local_addr()?;
    let temp_dir = TempDir::new("nodelite-agent-token-expired-test");
    let config_path = temp_dir.path().join("agent.toml");

    let config = test_config(local_addr);
    let identity = test_identity(&config);
    let collector = new_collector();
    let log_buffer = AgentLogBuffer::default();

    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
    let agent_task = tokio::spawn(run_forever(
        config,
        collector,
        identity,
        config_path,
        log_buffer,
        async move {
            let _ = shutdown_rx.await;
        },
    ));

    // 第一次连接:完成 WebSocket 握手,读取 Hello,然后下发 "token expired" 错误通知,
    // 触发 Agent 的 token 过期独立退避路径(首次 30s)。
    let (stream1, _) = tokio::select! {
        res = listener.accept() => res?,
        _ = tokio::time::sleep(Duration::from_secs(5)) => {
            panic!("timed out waiting for first connection");
        }
    };
    serve_token_expired_notice(stream1).await?;

    // 至此 Agent 已读到通知、返回 token 过期错误并断开连接,即将进入 30s 独立退避。
    // 现在才暂停时钟:握手 + 通知必须在真实时间下完成,否则 `tokio::time::pause()` 的自动
    // 推进会与真实 WebSocket 握手 I/O 抢跑(Windows IOCP 上尤甚),把 connect 超时打断。
    tokio::time::pause();

    // 常规退避(≤5s)在 6s 内必定重连;token 过期路径要等 30s,因此此处不得重连。
    let mut accept_fut = Box::pin(listener.accept());
    tokio::time::advance(Duration::from_secs(6)).await;
    assert!(
        futures::poll!(accept_fut.as_mut()).is_pending(),
        "agent reconnected within the normal window; the token-expired path must wait 30s",
    );

    // 推进越过 30s 后,token 过期退避到期,Agent 重连。
    tokio::time::advance(Duration::from_secs(30)).await;
    let (stream2, _) = accept_fut.await?;
    drop(stream2);

    let _ = shutdown_tx.send(());
    agent_task
        .await
        .expect("agent token-expired task should not panic")?;
    Ok(())
}

/// 在认证前完成一次最小 WebSocket 会话:读取 Hello 后立刻关闭连接,并等待对端
/// 处理 close。返回时 Agent 已离开握手流程,调用方可以安全切到虚拟时间验证退避。
async fn serve_pre_auth_disconnect(stream: TcpStream) -> Result<()> {
    let mut ws = accept_async(stream)
        .await
        .map_err(|error| anyhow!("ws handshake failed: {error}"))?;
    ws.next()
        .await
        .ok_or_else(|| anyhow!("connection closed before Hello"))?
        .map_err(|error| anyhow!("read Hello failed: {error}"))?;
    ws.close(None)
        .await
        .map_err(|error| anyhow!("close pre-auth websocket failed: {error}"))?;
    while let Some(frame) = ws.next().await {
        if matches!(frame, Ok(Message::Close(_)) | Err(_)) {
            break;
        }
    }
    Ok(())
}

/// 在已建立的 TCP 连接上完成 WebSocket 握手,读取 Agent 的 Hello,然后回送一条
/// `token expired` 错误通知,并等待 Agent 因 token 过期而主动断开后返回。
async fn serve_token_expired_notice(stream: TcpStream) -> Result<()> {
    let mut ws = accept_async(stream)
        .await
        .map_err(|error| anyhow!("ws handshake failed: {error}"))?;
    // 读取 Hello:内容无关紧要,只需确认 Agent 已进入会话循环。
    ws.next()
        .await
        .ok_or_else(|| anyhow!("connection closed before Hello"))?
        .map_err(|error| anyhow!("read Hello failed: {error}"))?;
    let notice = WireMessage::ServerNotice(ServerNoticeMessage {
        level: NoticeLevel::Error,
        code: Some(nodelite_proto::ServerNoticeCode::TokenExpired),
        message: "token expired".to_string(),
    });
    let payload =
        serde_json::to_string(&notice).map_err(|error| anyhow!("serialize notice: {error}"))?;
    ws.send(Message::Text(payload.into()))
        .await
        .map_err(|error| anyhow!("send notice failed: {error}"))?;
    // 同步点:Agent 读到 "token expired" 会立即返回错误并断开连接。读取直到流结束,确保
    // 返回时 Agent 确已处理完通知、进入退避路径,而不是仍卡在握手/读取的 I/O 上——这样
    // 调用方随后切到虚拟时间(pause)就不会再和真实 I/O 抢跑。
    while let Some(frame) = ws.next().await {
        if matches!(frame, Ok(Message::Close(_)) | Err(_)) {
            break;
        }
    }
    Ok(())
}
