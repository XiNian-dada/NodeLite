//! A small duplex buffer reproduces a peer that stops reading without relying on TCP buffer sizes.

use std::time::Duration;

use futures::StreamExt;
use tokio::io::{AsyncReadExt, DuplexStream, duplex};
use tokio::time::{Instant, sleep, timeout};
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::protocol::Role;

use super::{SendError, TimedSender};

type TestSender = TimedSender<futures::stream::SplitSink<WebSocketStream<DuplexStream>, Message>>;

async fn pair(send_timeout: Duration) -> (TestSender, DuplexStream) {
    let (stream, peer) = duplex(64);
    let socket = WebSocketStream::from_raw_socket(stream, Role::Client, None).await;
    let (sender, _) = socket.split();
    (TimedSender::new(sender, send_timeout), peer)
}

#[tokio::test(start_paused = true)]
async fn a_peer_that_never_reads_cannot_block_flush_forever() {
    let limit = Duration::from_secs(2);
    let (mut sender, _peer) = pair(limit).await;
    let started = Instant::now();
    let error = sender
        .send(Message::Text("x".repeat(1024).into()))
        .await
        .expect_err("the unread 64-byte pipe cannot flush a full frame");
    assert!(matches!(error, SendError::TimedOut));
    assert_eq!(started.elapsed(), limit);
}

#[tokio::test(start_paused = true)]
async fn a_slow_reader_can_complete_before_the_send_deadline() {
    let (mut sender, mut peer) = pair(Duration::from_secs(2)).await;
    let reader = tokio::spawn(async move {
        sleep(Duration::from_millis(250)).await;
        let mut bytes = Vec::new();
        peer.read_to_end(&mut bytes)
            .await
            .expect("read delayed frame");
        bytes
    });
    sender
        .send(Message::Text("x".repeat(1024).into()))
        .await
        .expect("slow send succeeds");
    drop(sender);
    assert!(reader.await.expect("reader task").len() > 1024);
}

#[tokio::test]
async fn a_pending_flush_is_cancellable_without_waiting_for_its_deadline() {
    let (mut sender, _peer) = pair(Duration::from_secs(3600)).await;
    let mut send = Box::pin(sender.send(Message::Text("x".repeat(1024).into())));
    assert!(futures::poll!(send.as_mut()).is_pending());
    let task = tokio::spawn(async move {
        let (mut sender, _peer) = pair(Duration::from_secs(3600)).await;
        sender.send(Message::Text("x".repeat(1024).into())).await
    });
    tokio::task::yield_now().await;
    task.abort();
    let error = timeout(Duration::from_millis(200), task)
        .await
        .expect("cancellation remains responsive")
        .expect_err("task was cancelled");
    assert!(error.is_cancelled());
}
