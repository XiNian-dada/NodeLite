//! Lab-only frame gates exercise real Agent durability windows without logging payloads.

use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

use anyhow::Result;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{OriginalUri, State};
use axum::routing::get;
use futures::{SinkExt, StreamExt};
use tokio::sync::Notify;
use tokio_tungstenite::tungstenite::Message as Upstream;
use tokio_util::sync::CancellationToken;

use nodelite_proto::WireMessage;

#[derive(Default)]
pub(super) struct Gate {
    mode: AtomicU8,
    pub reached: Notify,
    pub release: Notify,
}

impl Gate {
    pub fn arm(&self, mode: u8) {
        self.mode.store(mode, Ordering::SeqCst);
    }

    async fn pause(&self, mode: u8) -> bool {
        if self
            .mode
            .compare_exchange(mode, 0, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            self.reached.notify_one();
            self.release.notified().await;
            return true;
        }
        false
    }
}

pub(super) struct Proxy {
    pub url: String,
    pub gate: Arc<Gate>,
    stop: CancellationToken,
    task: tokio::task::JoinHandle<Result<(), std::io::Error>>,
}

impl Proxy {
    pub async fn start(upstream: std::net::SocketAddr) -> Result<Self> {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let url = format!("ws://{}/ws", listener.local_addr()?);
        let gate = Arc::new(Gate::default());
        let stop = CancellationToken::new();
        let app = axum::Router::new()
            .route("/ws", get(upgrade))
            .route("/ws/preflight", get(upgrade))
            .with_state((upstream, Arc::clone(&gate), stop.clone()));
        let cancelled = stop.clone();
        let task = tokio::spawn(async move {
            axum::serve(listener, app)
                .with_graceful_shutdown(cancelled.cancelled_owned())
                .await
        });
        Ok(Self {
            url,
            gate,
            stop,
            task,
        })
    }

    pub async fn close(self) -> Result<()> {
        self.stop.cancel();
        self.task.await??;
        Ok(())
    }
}

async fn upgrade(
    State((upstream, gate, stop)): State<(std::net::SocketAddr, Arc<Gate>, CancellationToken)>,
    OriginalUri(uri): OriginalUri,
    ws: WebSocketUpgrade,
) -> axum::response::Response {
    ws.on_upgrade(move |socket| async move {
        tokio::select! {
            _ = stop.cancelled() => {}
            _ = relay(socket, format!("ws://{upstream}{}", uri.path()), gate) => {}
        }
    })
}

async fn relay(mut agent: WebSocket, url: String, gate: Arc<Gate>) -> Result<()> {
    let (mut server, _) = tokio_tungstenite::connect_async(url).await?;
    loop {
        tokio::select! {
            frame = agent.next() => {
                let Some(frame) = frame else { return Ok(()) };
                let message = match frame? {
                    Message::Text(text) => {
                        let is_ack = matches!(serde_json::from_str::<WireMessage>(&text),
                            Ok(WireMessage::RefreshTokenAck(ack)) if ack.persisted);
                        if is_ack && gate.pause(2).await { return Ok(()) }
                        Upstream::Text(text.to_string().into())
                    }
                    Message::Binary(bytes) => Upstream::Binary(bytes),
                    Message::Ping(bytes) => Upstream::Ping(bytes),
                    Message::Pong(bytes) => Upstream::Pong(bytes),
                    Message::Close(_) => return Ok(()),
                };
                server.send(message).await?;
            }
            frame = server.next() => {
                let Some(frame) = frame else { return Ok(()) };
                let message = match frame? {
                    Upstream::Text(text) => {
                        let is_refresh = matches!(serde_json::from_str::<WireMessage>(&text),
                            Ok(WireMessage::RefreshTokenResponse(_)));
                        if is_refresh && gate.pause(1).await { return Ok(()) }
                        Message::Text(text.to_string().into())
                    }
                    Upstream::Binary(bytes) => Message::Binary(bytes),
                    Upstream::Ping(bytes) => Message::Ping(bytes),
                    Upstream::Pong(bytes) => Message::Pong(bytes),
                    Upstream::Close(_) => return Ok(()),
                    Upstream::Frame(_) => continue,
                };
                agent.send(message).await?;
            }
        }
    }
}
