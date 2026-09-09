//! Real WebSocket regressions for silent peers, recovery and process shutdown.

use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use futures::{SinkExt, StreamExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;
use tokio::time::{interval, sleep, timeout};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{WebSocketStream, accept_async};

use nodelite_agent::collector::new_collector;
use nodelite_agent::session::{AgentLogBuffer, SessionError, run_forever, run_session};
use nodelite_proto::{AgentConfig, NoticeLevel, PingMessage, ServerNoticeMessage, WireMessage};

mod common;
use common::{TempDir, test_config, test_identity};

fn spawn_once(mut config: AgentConfig) -> JoinHandle<Result<(), SessionError>> {
    tokio::spawn(async move {
        let dir = TempDir::new("nodelite-session-deadline");
        let identity = test_identity(&config);
        run_session(
            &mut config,
            &mut new_collector(),
            &identity,
            &dir.path().join("agent.toml"),
            &mut AgentLogBuffer::default(),
        )
        .await
    })
}

async fn accept_hello(listener: &TcpListener) -> Result<WebSocketStream<TcpStream>> {
    timeout(Duration::from_secs(8), async {
        let (stream, _) = listener.accept().await?;
        let mut socket = accept_async(stream).await?;
        let frame = socket.next().await.context("agent did not send Hello")??;
        let Message::Text(text) = frame else {
            return Err(anyhow!("Hello was not text"));
        };
        assert!(matches!(
            serde_json::from_str::<WireMessage>(&text)?,
            WireMessage::Hello(_)
        ));
        Ok(socket)
    })
    .await
    .context("timed out accepting Agent")?
}

async fn authenticate(socket: &mut WebSocketStream<TcpStream>) -> Result<()> {
    let notice = WireMessage::ServerNotice(ServerNoticeMessage {
        level: NoticeLevel::Info,
        code: None,
        message: "authenticated".into(),
    });
    socket
        .send(Message::Text(serde_json::to_string(&notice)?.into()))
        .await?;
    Ok(())
}

#[tokio::test]
async fn a_silent_upgrade_reconnects_and_a_slow_healthy_peer_recovers() -> Result<()> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let mut config = test_config(listener.local_addr()?);
    config.auth_timeout_secs = 1;
    config.send_timeout_secs = 1;
    config.inbound_timeout_secs = 2;
    config.report_interval_secs = 1;
    let identity = test_identity(&config);
    let dir = TempDir::new("nodelite-deadline-reconnect");
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
    let agent = tokio::spawn(run_forever(
        config,
        new_collector(),
        identity,
        dir.path().join("agent.toml"),
        AgentLogBuffer::default(),
        async {
            let _ = shutdown_rx.await;
        },
    ));

    let _silent = accept_hello(&listener).await?;
    let mut recovered = accept_hello(&listener).await?;
    sleep(Duration::from_millis(250)).await;
    authenticate(&mut recovered).await?;
    let mut metrics = 0;
    for nonce in 0..5 {
        sleep(Duration::from_millis(500)).await;
        recovered
            .send(Message::Text(
                serde_json::to_string(&WireMessage::Ping(PingMessage { nonce }))?.into(),
            ))
            .await?;
        timeout(Duration::from_secs(1), async {
            loop {
                let frame = recovered
                    .next()
                    .await
                    .context("recovered connection closed")??;
                if let Message::Text(text) = frame {
                    match serde_json::from_str::<WireMessage>(&text)? {
                        WireMessage::Pong(pong) if pong.nonce == nonce => {
                            return Ok::<(), anyhow::Error>(());
                        }
                        WireMessage::Metrics(_) => metrics += 1,
                        _ => {}
                    }
                }
            }
        })
        .await
        .context("slow healthy link stopped responding")??;
    }
    assert!(metrics > 0, "reporting must resume after timeout recovery");
    shutdown_tx.send(()).expect("agent still running");
    timeout(Duration::from_secs(1), agent)
        .await
        .context("shutdown stalled")???;
    Ok(())
}

#[tokio::test]
async fn pre_authentication_pings_do_not_extend_the_authentication_deadline() -> Result<()> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let mut config = test_config(listener.local_addr()?);
    config.auth_timeout_secs = 1;
    let agent = spawn_once(config);
    let mut socket = accept_hello(&listener).await?;
    let pinger = tokio::spawn(async move {
        let mut ticker = interval(Duration::from_millis(100));
        let mut sent = 0;
        loop {
            tokio::select! {
                _ = ticker.tick() => {
                    if socket.send(Message::Ping(Vec::new().into())).await.is_err() { break; }
                    sent += 1;
                }
                frame = socket.next() => {
                    if !matches!(frame, Some(Ok(Message::Pong(_)))) { break; }
                }
            }
        }
        sent
    });
    let error = timeout(Duration::from_secs(3), agent)
        .await??
        .expect_err("auth must time out");
    assert!(!error.established_session);
    assert!(
        error
            .to_string()
            .contains("authentication response timed out")
    );
    assert!(
        pinger.await? >= 2,
        "peer kept sending while authentication was pending"
    );
    Ok(())
}

#[tokio::test]
async fn outgoing_metrics_do_not_mask_missing_inbound_activity() -> Result<()> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let mut config = test_config(listener.local_addr()?);
    config.inbound_timeout_secs = 1;
    config.report_interval_secs = 1;
    let agent = spawn_once(config);
    let mut socket = accept_hello(&listener).await?;
    authenticate(&mut socket).await?;
    let reader = tokio::spawn(async move {
        let mut metrics = 0;
        while let Some(Ok(Message::Text(text))) = socket.next().await {
            if matches!(
                serde_json::from_str::<WireMessage>(&text),
                Ok(WireMessage::Metrics(_))
            ) {
                metrics += 1;
            }
        }
        metrics
    });
    let error = timeout(Duration::from_secs(3), agent)
        .await??
        .expect_err("inbound must time out");
    assert!(error.established_session);
    assert!(error.to_string().contains("inbound activity timed out"));
    assert!(
        reader.await? > 0,
        "sending metrics must not renew the inbound deadline"
    );
    Ok(())
}

#[cfg(unix)]
#[tokio::test]
async fn sigterm_exits_while_waiting_for_a_silent_server() -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    use std::process::Stdio;

    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let dir = TempDir::new("nodelite-sigterm-deadline");
    let path = dir.path().join("agent.toml");
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700))?;
    std::fs::write(
        &path,
        format!(
            "[agent]\nnode_id = \"signal-test\"\nnode_label = \"Signal Test\"\nserver = \"ws://{}/ws\"\ntoken = \"local-test-only\"\nauth_timeout_secs = 30\n",
            listener.local_addr()?
        ),
    )?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    let mut child = tokio::process::Command::new(env!("CARGO_BIN_EXE_nodelite-agent"))
        .arg("--config")
        .arg(&path)
        .kill_on_drop(true)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    let _socket = accept_hello(&listener).await?;
    let status = tokio::process::Command::new("kill")
        .arg("-TERM")
        .arg(
            child
                .id()
                .context("agent process stopped early")?
                .to_string(),
        )
        .status()
        .await?;
    assert!(status.success());
    let status = timeout(Duration::from_secs(1), child.wait())
        .await
        .context("SIGTERM did not interrupt the session")??;
    assert!(status.success(), "agent should shut down cleanly");
    Ok(())
}
