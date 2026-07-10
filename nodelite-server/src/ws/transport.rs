//! WebSocket transport buffer budgets shared by Agent and browser endpoints.

use axum::extract::ws::WebSocketUpgrade;

const AGENT_READ_BUFFER_BYTES: usize = 8 * 1024;
const BROWSER_READ_BUFFER_BYTES: usize = 4 * 1024;
const WRITE_BUFFER_BYTES: usize = 8 * 1024;
const AGENT_MAX_WRITE_BUFFER_BYTES: usize = 128 * 1024;
const BROWSER_MAX_WRITE_BUFFER_BYTES: usize = 32 * 1024 * 1024;

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

#[cfg(test)]
mod tests {
    use std::io::{Error, ErrorKind, Read, Write};

    use tokio_tungstenite::tungstenite::error::Error as WsError;
    use tokio_tungstenite::tungstenite::protocol::{Message, Role, WebSocket, WebSocketConfig};

    use super::*;

    struct BlockedWriter;

    impl Read for BlockedWriter {
        fn read(&mut self, _buffer: &mut [u8]) -> std::io::Result<usize> {
            Err(Error::from(ErrorKind::WouldBlock))
        }
    }

    impl Write for BlockedWriter {
        fn write(&mut self, _buffer: &[u8]) -> std::io::Result<usize> {
            Err(Error::from(ErrorKind::WouldBlock))
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Err(Error::from(ErrorKind::WouldBlock))
        }
    }

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

    #[test]
    fn write_buffers_reject_growth_after_transport_blocks() {
        for (peer, payload_bytes) in [
            (WebSocketPeer::Agent, 64 * 1024),
            (WebSocketPeer::Browser, 16 * 1024 * 1024),
        ] {
            let limits = websocket_config(peer, 64 * 1024);
            let config = WebSocketConfig::default()
                .read_buffer_size(limits.read_buffer_size)
                .write_buffer_size(limits.write_buffer_size)
                .max_write_buffer_size(limits.max_write_buffer_size)
                .max_frame_size(Some(limits.max_message_bytes))
                .max_message_size(Some(limits.max_message_bytes));
            let mut socket = WebSocket::from_raw_socket(BlockedWriter, Role::Server, Some(config));
            let payload = vec![0_u8; payload_bytes];

            let first = socket.write(Message::Binary(payload.clone().into()));
            assert!(
                matches!(first, Err(WsError::Io(error)) if error.kind() == ErrorKind::WouldBlock)
            );

            let second = socket.write(Message::Binary(payload.into()));
            assert!(matches!(second, Err(WsError::WriteBufferFull(_))));
        }
    }
}
