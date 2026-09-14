//! Negotiated compression preserves legacy sessions and the existing plaintext size boundary.

use super::*;
use crate::test_support::{
    TestNode, TestSocket, fake_snapshot, send_wire_message, synthetic_identity,
};
use anyhow::Context;
use nodelite_proto::{MetricsMessage, ServerNoticeCode, compression::encode_metrics};

async fn connect(server: &TestServer, node: &TestNode, compressed: bool) -> Result<TestSocket> {
    let (mut socket, _) = connect_async(format!("ws://{}/ws", server.addr)).await?;
    send_wire_message(
        &mut socket,
        &WireMessage::Hello(HelloMessage {
            supports_metrics_zlib: compressed,
            protocol_version: WIRE_PROTOCOL_VERSION,
            token: node.token.clone(),
            identity: synthetic_identity(
                &node.node_id,
                &node.node_label,
                "test",
                None,
                "compression",
            ),
        }),
    )
    .await?;
    let frame = tokio::time::timeout(TEST_TIMEOUT, socket.next())
        .await?
        .context("auth frame")??;
    let Message::Text(text) = frame else {
        anyhow::bail!("expected JSON auth notice")
    };
    let WireMessage::ServerNotice(notice) = serde_json::from_str(&text)? else {
        anyhow::bail!("expected auth notice")
    };
    assert_eq!(notice.message, "authenticated");
    assert_eq!(
        notice.code,
        compressed.then_some(ServerNoticeCode::MetricsZlibV1)
    );
    Ok(socket)
}

async fn expect_closed(socket: &mut TestSocket) -> Result<()> {
    tokio::time::timeout(TEST_TIMEOUT, async {
        loop {
            match socket.next().await {
                None | Some(Err(_)) | Some(Ok(Message::Close(_))) => return,
                _ => {}
            }
        }
    })
    .await?;
    Ok(())
}

#[tokio::test]
async fn compressed_metrics_roundtrip_and_legacy_text_fallback() -> Result<()> {
    let server = TestServer::start().await?;
    let node = server.issue_node("compressed", "Compressed").await?;
    let mut socket = connect(&server, &node, true).await?;
    let metrics = MetricsMessage {
        snapshot: fake_snapshot(1),
    };
    socket
        .send(Message::Binary(encode_metrics(&metrics)?.into()))
        .await?;
    server
        .wait_for_node_uptime(&node.node_id, 1, TEST_TIMEOUT)
        .await?;
    send_wire_message(
        &mut socket,
        &WireMessage::Metrics(MetricsMessage {
            snapshot: fake_snapshot(2),
        }),
    )
    .await?;
    server
        .wait_for_node_uptime(&node.node_id, 2, TEST_TIMEOUT)
        .await?;
    socket.close(None).await?;
    server.shutdown().await
}

#[tokio::test]
async fn compressed_metrics_require_authenticated_negotiation() -> Result<()> {
    let server = TestServer::start().await?;
    let node = server.issue_node("legacy", "Legacy").await?;
    let metrics = encode_metrics(&MetricsMessage {
        snapshot: fake_snapshot(1),
    })?;
    let mut legacy = connect(&server, &node, false).await?;
    legacy.send(Message::Binary(metrics.clone().into())).await?;
    expect_closed(&mut legacy).await?;
    let (mut unauthenticated, _) = connect_async(format!("ws://{}/ws", server.addr)).await?;
    unauthenticated
        .send(Message::Binary(metrics.into()))
        .await?;
    expect_closed(&mut unauthenticated).await?;
    server.shutdown().await
}

#[tokio::test]
async fn compressed_metrics_cannot_bypass_the_plaintext_size_limit() -> Result<()> {
    let server = TestServer::start().await?;
    let node = server
        .issue_node("oversized-compressed", "Oversized")
        .await?;
    let mut socket = connect(&server, &node, true).await?;
    let mut snapshot = fake_snapshot(1);
    snapshot.disks[0].device = "x".repeat(MAX_MESSAGE_BYTES + 1);
    let encoded = encode_metrics(&MetricsMessage { snapshot })?;
    assert!(encoded.len() < MAX_MESSAGE_BYTES);
    socket.send(Message::Binary(encoded.into())).await?;
    expect_closed(&mut socket).await?;
    server.shutdown().await
}
