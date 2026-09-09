//! Bound every WebSocket send, including the flush hidden inside SinkExt::send.

use std::time::Duration;

use futures::{Sink, SinkExt};
use thiserror::Error;
use tokio::time::timeout;
use tokio_tungstenite::tungstenite::{Error as WebSocketError, Message};

#[derive(Debug, Error)]
pub(super) enum SendError {
    #[error("websocket send timed out")]
    TimedOut,
    #[error("websocket send failed: {0}")]
    Transport(#[from] WebSocketError),
}

pub(super) struct TimedSender<S> {
    sink: S,
    timeout: Duration,
}

impl<S: Sink<Message, Error = WebSocketError> + Unpin> TimedSender<S> {
    pub(super) fn new(sink: S, timeout: Duration) -> Self {
        Self { sink, timeout }
    }

    pub(super) async fn send(&mut self, message: Message) -> Result<(), SendError> {
        // A timed-out partial frame is never reused: the caller drops the whole session.
        timeout(self.timeout, self.sink.send(message))
            .await
            .map_err(|_| SendError::TimedOut)??;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
