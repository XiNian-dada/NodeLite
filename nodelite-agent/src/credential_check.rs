//! Upgrade probes use a distinct endpoint so they cannot displace the running Agent session.

use std::time::Duration;

use futures::{SinkExt, StreamExt};
use nodelite_proto::{
    AgentConfig, HelloMessage, NodeIdentity, NoticeLevel, WIRE_PROTOCOL_VERSION, WireMessage,
};
use thiserror::Error;
use tokio_tungstenite::tungstenite::Message;

#[derive(Debug, Error)]
pub enum CredentialCheckError {
    #[error("invalid credential-check URL")]
    InvalidUrl,
    #[error("credential check timed out; running Agent was not replaced")]
    Timeout,
    #[error(
        "credential-check endpoint unavailable; upgrade the server first and verify connectivity"
    )]
    Unavailable,
    #[error("saved Agent credentials were rejected; reissue credentials before upgrading")]
    Rejected,
}

/// Checks disk-loaded credentials without registering a node or requesting a token rotation.
pub async fn check_credentials(
    config: &AgentConfig,
    identity: NodeIdentity,
) -> Result<(), CredentialCheckError> {
    let mut url = url::Url::parse(&config.server).map_err(|_| CredentialCheckError::InvalidUrl)?;
    let path = format!("{}/preflight", url.path().trim_end_matches('/'));
    url.set_path(&path);
    url.set_query(None);
    let duration = Duration::from_secs(
        config
            .connect_timeout_secs
            .saturating_add(config.auth_timeout_secs),
    );
    tokio::time::timeout(duration, async {
        let (mut socket, _) = crate::tls::connect(
            url.as_str(),
            crate::session::incoming_ws_config(config.max_incoming_message_bytes),
        )
        .await
        .map_err(|_| CredentialCheckError::Unavailable)?;
        let hello = WireMessage::Hello(HelloMessage {
            supports_agent_updates: false,
            supports_metrics_zlib: false,
            supports_token_refresh_ack: true,
            protocol_version: WIRE_PROTOCOL_VERSION,
            token: config.token.clone(),
            identity,
        });
        let text = serde_json::to_string(&hello).map_err(|_| CredentialCheckError::Unavailable)?;
        socket
            .send(Message::Text(text.into()))
            .await
            .map_err(|_| CredentialCheckError::Unavailable)?;
        while let Some(frame) = socket.next().await {
            match frame.map_err(|_| CredentialCheckError::Unavailable)? {
                Message::Text(text) => {
                    return match serde_json::from_str::<WireMessage>(&text) {
                        Ok(WireMessage::ServerNotice(notice))
                            if notice.level == NoticeLevel::Info
                                && notice.message == "credentials valid" =>
                        {
                            Ok(())
                        }
                        _ => Err(CredentialCheckError::Rejected),
                    };
                }
                Message::Ping(bytes) => socket
                    .send(Message::Pong(bytes))
                    .await
                    .map_err(|_| CredentialCheckError::Unavailable)?,
                Message::Close(_) => break,
                _ => {}
            }
        }
        Err(CredentialCheckError::Unavailable)
    })
    .await
    .map_err(|_| CredentialCheckError::Timeout)?
}
