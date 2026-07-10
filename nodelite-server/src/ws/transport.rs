//! WebSocket transport buffer budgets shared by Agent and browser endpoints.

use std::fmt::Display;
use std::time::Duration;

use anyhow::anyhow;
use axum::extract::ws::{Message, WebSocketUpgrade};
use futures::{Sink, SinkExt};

const AGENT_READ_BUFFER_BYTES: usize = 8 * 1024;
const BROWSER_READ_BUFFER_BYTES: usize = 4 * 1024;
const WRITE_BUFFER_BYTES: usize = 8 * 1024;
const AGENT_MAX_WRITE_BUFFER_BYTES: usize = 128 * 1024;
const BROWSER_MAX_WRITE_BUFFER_BYTES: usize = 32 * 1024 * 1024;
/// Slow peers may apply backpressure briefly, but must not retain a session permit forever.
const SEND_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone, Copy)]
pub(super) enum WebSocketPeer {
    Agent,
    Browser,
}

#[derive(Clone, Copy)]
struct WebSocketTransportConfig {
    read_buffer_size: usize,
    write_buffer_size: usize,
    max_write_buffer_size: usize,
    max_message_bytes: usize,
}

/// Apply bounded buffers without coupling the eager read allocation to the message limit.
pub(super) fn configure_upgrade<F>(
    ws: WebSocketUpgrade<F>,
    peer: WebSocketPeer,
    max_message_bytes: usize,
) -> WebSocketUpgrade<F> {
    let config = websocket_config(peer, max_message_bytes);
    ws.read_buffer_size(config.read_buffer_size)
        .write_buffer_size(config.write_buffer_size)
        .max_write_buffer_size(config.max_write_buffer_size)
        .max_frame_size(config.max_message_bytes)
        .max_message_size(config.max_message_bytes)
}

fn websocket_config(peer: WebSocketPeer, max_message_bytes: usize) -> WebSocketTransportConfig {
    let (read_buffer_size, max_write_buffer_size) = match peer {
        // Agent metrics can approach the 64 KiB protocol limit, but tungstenite reassembles
        // frames across reads, so 8 KiB retains throughput without 128 KiB per connection.
        WebSocketPeer::Agent => (AGENT_READ_BUFFER_BYTES, AGENT_MAX_WRITE_BUFFER_BYTES),
        // Browsers only send a tiny application-level ping. Server InitialState messages can
        // cover the 1000-node target; 32 MiB leaves about 32 KiB of metadata per target node.
        WebSocketPeer::Browser => (BROWSER_READ_BUFFER_BYTES, BROWSER_MAX_WRITE_BUFFER_BYTES),
    };
    WebSocketTransportConfig {
        read_buffer_size,
        write_buffer_size: WRITE_BUFFER_BYTES,
        max_write_buffer_size,
        max_message_bytes,
    }
}

pub(super) async fn send_message<S>(
    sender: &mut S,
    message: Message,
    operation: &'static str,
) -> anyhow::Result<()>
where
    S: Sink<Message> + Unpin,
    S::Error: Display,
{
    send_message_with_timeout(sender, message, operation, SEND_TIMEOUT).await
}

async fn send_message_with_timeout<S, M>(
    sender: &mut S,
    message: M,
    operation: &'static str,
    timeout_duration: Duration,
) -> anyhow::Result<()>
where
    S: Sink<M> + Unpin,
    S::Error: Display,
{
    match tokio::time::timeout(timeout_duration, sender.send(message)).await {
        Ok(Ok(())) => Ok(()),
        Ok(Err(error)) => Err(anyhow!("{operation}: {error}")),
        Err(_) => Err(anyhow!(
            "{operation}: timed out after {}ms",
            timeout_duration.as_millis()
        )),
    }
}

#[cfg(test)]
mod tests {
    use tokio::io::duplex;
    use tokio_tungstenite::WebSocketStream;
    use tokio_tungstenite::tungstenite::protocol::{Message, Role};

    use super::*;

    #[test]
    fn transport_budgets_match_each_peer_workload() {
        let agent = websocket_config(WebSocketPeer::Agent, 64 * 1024);
        assert_eq!(agent.read_buffer_size, 8 * 1024);
        assert_eq!(agent.write_buffer_size, 8 * 1024);
        assert_eq!(agent.max_write_buffer_size, 128 * 1024);
        assert_eq!(agent.max_message_bytes, 64 * 1024);

        let browser = websocket_config(WebSocketPeer::Browser, 64 * 1024);
        assert_eq!(browser.read_buffer_size, 4 * 1024);
        assert_eq!(browser.write_buffer_size, 8 * 1024);
        assert_eq!(browser.max_write_buffer_size, 32 * 1024 * 1024);
    }

    #[tokio::test]
    async fn async_send_times_out_when_peer_stops_reading() {
        let (stream, _blocked_peer) = duplex(1);
        let mut socket = WebSocketStream::from_raw_socket(stream, Role::Server, None).await;

        let error = send_message_with_timeout(
            &mut socket,
            Message::Binary(vec![0_u8; 64 * 1024].into()),
            "test websocket send",
            Duration::from_millis(20),
        )
        .await
        .expect_err("blocked async websocket should time out");

        assert_eq!(
            error.to_string(),
            "test websocket send: timed out after 20ms"
        );
    }
}
