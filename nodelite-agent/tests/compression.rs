//! A real Agent opts in, then uses binary Metrics only after a compatible authentication notice.

use std::time::Duration;

use anyhow::{Context, Result};
use futures::{SinkExt, StreamExt};
use tokio::net::TcpListener;
use tokio::time::timeout;
use tokio_tungstenite::{accept_async, tungstenite::Message};

use nodelite_agent::collector::new_collector;
use nodelite_agent::session::{AgentLogBuffer, run_session};
use nodelite_proto::{
    NoticeLevel, ServerNoticeCode, ServerNoticeMessage, WireMessage, compression::decode_metrics,
};

mod common;
use common::{TempDir, test_config, test_identity};

#[tokio::test]
async fn real_agent_negotiates_compression_and_falls_back_with_older_servers() -> Result<()> {
    for negotiated in [false, true] {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let mut config = test_config(listener.local_addr()?);
        let task = tokio::spawn(async move {
            let dir = TempDir::new("compression-agent");
            let identity = test_identity(&config);
            run_session(
                &mut config,
                &mut new_collector(),
                &identity,
                &dir.path().join("agent.toml"),
                &mut AgentLogBuffer::default(),
            )
            .await
        });
        timeout(Duration::from_secs(10), async {
            let (stream, _) = listener.accept().await?;
            let mut socket = accept_async(stream).await?;
            let frame = socket.next().await.context("Agent Hello")??;
            let Message::Text(text) = frame else {
                anyhow::bail!("Hello must stay JSON")
            };
            let WireMessage::Hello(hello) = serde_json::from_str(&text)? else {
                anyhow::bail!("expected Hello")
            };
            assert!(hello.supports_metrics_zlib);
            let notice = WireMessage::ServerNotice(ServerNoticeMessage {
                level: NoticeLevel::Info,
                code: negotiated.then_some(ServerNoticeCode::MetricsZlibV1),
                message: "authenticated".into(),
            });
            socket
                .send(Message::Text(serde_json::to_string(&notice)?.into()))
                .await?;
            loop {
                match socket.next().await.context("Agent metrics")?? {
                    Message::Binary(bytes) => {
                        assert!(negotiated);
                        decode_metrics(&bytes, 1024 * 1024)?;
                        break;
                    }
                    Message::Text(text)
                        if matches!(serde_json::from_str(&text)?, WireMessage::Metrics(_)) =>
                    {
                        assert!(!negotiated);
                        break;
                    }
                    _ => {}
                }
            }
            socket.close(None).await?;
            anyhow::Ok(())
        })
        .await??;
        assert!(timeout(Duration::from_secs(2), task).await??.is_err());
    }
    Ok(())
}
