//! WebSocket 发送超时封装模块。
//!
//! 确保每一次 WebSocket 发送及底层隐式 flush 都受设定的超时约束，防止 TCP 拥塞拖垮 Agent。

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
        // Why: 发送超时的部分帧无法安全复用，必须销毁整个底层会话以重建连接，防止帧错位损坏协议流。
        timeout(self.timeout, self.sink.send(message))
            .await
            .map_err(|_| SendError::TimedOut)??;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
