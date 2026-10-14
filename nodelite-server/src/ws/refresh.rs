//! 在线 token 有效性检查与续期逻辑。

use anyhow::{Result, anyhow};
use axum::extract::ws::{Message, WebSocket};
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use tracing::{info, warn};

use super::ActiveSession;
use super::transport::send_message;
use crate::AppState;
use crate::registry::renewal::PreparedToken;
use crate::registry::{NodeRegistry, RegistryTokenStatus};
use crate::state::{SessionRefreshPermit, SessionRefreshReply};

pub(super) struct PendingRefresh {
    candidate: PreparedToken,
    retry_at: tokio::time::Instant,
    pub(super) reply: Option<(
        // Dropping an unfinished reply must release the permit before its sender.
        SessionRefreshPermit,
        tokio::sync::oneshot::Sender<Result<SessionRefreshReply, String>>,
    )>,
}

/// Token 距离过期不足该天数时,服务端在已认证会话内主动续期并下发新 token。
pub(crate) const AGENT_TOKEN_REFRESH_BEFORE_EXPIRY_DAYS: i64 = 7;

pub(crate) async fn handle_refresh_request(
    state: &AppState,
    session: &mut ActiveSession,
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    request: nodelite_proto::RefreshTokenRequestMessage,
) -> Result<super::LoopAction, super::ProtocolError> {
    if !ensure_current_token(
        state,
        session,
        "disconnecting session after token expiry before refresh",
    )
    .await
    {
        return Ok(super::LoopAction::Break);
    }
    if request.node_id != session.node_id {
        warn!(
            session_node_id = %session.node_id,
            client_supplied_node_id = %request.node_id,
            "ignoring client-supplied node_id in refresh_token_request",
        );
    }
    match refresh_session_token(sender, &state.registry, session, "agent-request").await {
        Ok(_) => {}
        Err(error) => {
            warn!(node_id = %session.node_id, error = ?error, "failed to refresh token");
            let notice =
                nodelite_proto::WireMessage::ServerNotice(nodelite_proto::ServerNoticeMessage {
                    level: nodelite_proto::NoticeLevel::Error,
                    code: None,
                    message: "Failed to refresh token".to_string(),
                });
            let payload = serde_json::to_string(&notice)
                .map_err(|error| anyhow!("failed to serialize notice: {error}"))?;
            send_message(
                sender,
                Message::Text(payload.into()),
                "failed to send notice",
            )
            .await?;
        }
    }
    Ok(super::LoopAction::Continue)
}

pub(crate) async fn ensure_current_token(
    state: &AppState,
    session: &mut ActiveSession,
    log_message: &str,
) -> bool {
    if session.registry_revision == state.registry.registry_revision()
        && session
            .token_expires_at
            .map(|expires_at| Utc::now() < expires_at)
            .unwrap_or(true)
    {
        return true;
    }

    let Ok(status) = state
        .registry
        .session_token_status(
            &session.node_id,
            &session.session_token,
            session.session_generation,
        )
        .await
    else {
        warn!(node_id = %session.node_id, "{log_message}");
        return false;
    };
    if !apply_current_token_status(session, status) {
        warn!(node_id = %session.node_id, "{log_message}");
        return false;
    }
    true
}

pub(crate) async fn should_refresh_agent_token(
    registry: &NodeRegistry,
    session: &mut ActiveSession,
) -> Result<bool> {
    if !session.supports_token_refresh_ack || tokio::time::Instant::now() < session.refresh_retry_at
    {
        return Ok(false);
    }
    if let Some(pending) = &mut session.pending_refresh {
        if tokio::time::Instant::now() < pending.retry_at {
            return Ok(false);
        }
        if let Some((permit, reply)) = pending.reply.take() {
            permit.complete(
                reply,
                Err("Agent did not confirm durable token storage".to_string()),
            );
        }
        return Ok(true);
    }
    let refresh_after = Utc::now() + ChronoDuration::days(AGENT_TOKEN_REFRESH_BEFORE_EXPIRY_DAYS);
    if session.registry_revision == registry.registry_revision() {
        return Ok(token_needs_refresh(session.token_expires_at, refresh_after));
    }

    let Some(status) = registry
        .token_status(&session.node_id, session.session_generation)
        .await
    else {
        return Ok(true);
    };
    if !apply_current_token_status(session, status) {
        return Ok(false);
    }

    Ok(token_needs_refresh(session.token_expires_at, refresh_after))
}

fn apply_current_token_status(session: &mut ActiveSession, status: RegistryTokenStatus) -> bool {
    if status.generation != session.session_generation {
        return false;
    }
    session.token_expires_at = status.token_expires_at;
    session.registry_revision = status.registry_revision;
    true
}

fn token_needs_refresh(
    token_expires_at: Option<DateTime<Utc>>,
    refresh_after: DateTime<Utc>,
) -> bool {
    token_expires_at
        .map(|expires_at| expires_at <= refresh_after)
        .unwrap_or(true)
}

/// Staging is not activation: the current token remains valid until a durable acknowledgement.
pub(crate) async fn refresh_session_token(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    registry: &NodeRegistry,
    session: &mut ActiveSession,
    trigger: &str,
) -> Result<DateTime<Utc>> {
    if !session.supports_token_refresh_ack {
        anyhow::bail!("Agent upgrade required for durable token renewal");
    }
    session.refresh_retry_at = tokio::time::Instant::now() + std::time::Duration::from_secs(60);
    if session.pending_refresh.is_none() {
        let candidate = registry
            .prepare_token_refresh(&session.node_id, session.session_generation)
            .await?;
        session.pending_refresh = Some(PendingRefresh {
            candidate,
            retry_at: tokio::time::Instant::now(),
            reply: None,
        });
    }
    let pending = session
        .pending_refresh
        .as_mut()
        .ok_or_else(|| anyhow!("missing staged refresh"))?;
    let expires_at = pending.candidate.expires_at;
    info!(
        node_id = %session.node_id,
        trigger,
        expires_at = %expires_at.to_rfc3339(),
        generation = pending.candidate.generation,
        "staged agent token awaiting durable confirmation",
    );
    let response = nodelite_proto::WireMessage::RefreshTokenResponse(
        nodelite_proto::RefreshTokenResponseMessage {
            new_token: pending.candidate.token.clone(),
            refresh_id: Some(pending.candidate.refresh_id.clone()),
            expires_at: expires_at.to_rfc3339(),
        },
    );
    let payload = serde_json::to_string(&response)
        .map_err(|error| anyhow!("failed to serialize token refresh response: {error}"))?;
    send_message(
        sender,
        Message::Text(payload.into()),
        "failed to send token refresh response",
    )
    .await?;
    pending.retry_at = tokio::time::Instant::now() + std::time::Duration::from_secs(60);
    Ok(expires_at)
}

pub(super) async fn handle_refresh_ack(
    state: &AppState,
    session: &mut ActiveSession,
    ack: nodelite_proto::RefreshTokenAckMessage,
) -> Result<super::LoopAction, super::ProtocolError> {
    crate::sanitize::validate_refresh_id(&ack.refresh_id)
        .map_err(|e| super::ProtocolError::Client(e.to_string()))?;
    if !state
        .shared
        .is_current_session(&session.node_id, session.session_id)
        .await
        || !ensure_current_token(state, session, "rejecting stale refresh acknowledgement").await
    {
        return Ok(super::LoopAction::Break);
    }
    let Some(pending) = session.pending_refresh.as_ref() else {
        return Ok(super::LoopAction::Continue);
    };
    if pending.candidate.refresh_id != ack.refresh_id {
        return Ok(super::LoopAction::Continue);
    }
    let Some(mut pending) = session.pending_refresh.take() else {
        return Ok(super::LoopAction::Continue);
    };
    if !ack.persisted {
        let reply = pending.reply.take();
        warn!(node_id = %session.node_id, "Agent could not persist token; current credential retained");
        session.pending_refresh = Some(pending);
        if let Some((permit, reply)) = reply {
            permit.complete(
                reply,
                Err(
                    "Agent could not persist the token; current credential was retained"
                        .to_string(),
                ),
            );
        }
        return Ok(super::LoopAction::Continue);
    }
    let (generation, expires_at) = state
        .registry
        .commit_token_refresh(&session.node_id, &ack.refresh_id)
        .await
        .map_err(anyhow::Error::from)?;
    apply_committed_refresh(session, pending.candidate.token, generation, expires_at);
    if let Some((permit, reply)) = pending.reply.take() {
        permit.complete(
            reply,
            Ok(SessionRefreshReply {
                token_expires_at: expires_at,
            }),
        );
    }
    info!(node_id = %session.node_id, "activated durably persisted Agent token");
    Ok(super::LoopAction::Continue)
}

pub(super) fn apply_committed_refresh(
    session: &mut ActiveSession,
    token: String,
    generation: u64,
    expires_at: DateTime<Utc>,
) {
    session.session_generation = generation;
    session.session_token = token;
    session.token_expires_at = Some(expires_at);
    // An operator rotation may have advanced the registry after this commit.
    session.registry_revision = 0;
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use chrono::Utc;
    use tokio::io::duplex;
    use tokio_tungstenite::WebSocketStream;
    use tokio_tungstenite::tungstenite::protocol::{Message as TungsteniteMessage, Role};

    use super::{
        AGENT_TOKEN_REFRESH_BEFORE_EXPIRY_DAYS, ActiveSession, RegistryTokenStatus,
        apply_current_token_status, token_needs_refresh,
    };
    use crate::registry::{IssueNodeRequest, NodeRegistry, issue_node};
    use crate::ws::transport::send_message_with_timeout;

    fn session() -> ActiveSession {
        ActiveSession {
            metrics_zlib: false,
            supports_token_refresh_ack: true,
            pending_refresh: None,
            refresh_retry_at: tokio::time::Instant::now(),
            node_id: "hk-01".to_string(),
            node_label: "Hong Kong 01".to_string(),
            agent_protocol_version: nodelite_proto::WIRE_PROTOCOL_VERSION,
            session_id: 7,
            session_token: "secret".to_string(),
            session_generation: 3,
            token_expires_at: None,
            registry_revision: 11,
        }
    }

    fn identity_for(node_id: &str) -> nodelite_proto::NodeIdentity {
        nodelite_proto::NodeIdentity {
            node_id: node_id.to_string(),
            node_label: node_id.to_string(),
            hostname: format!("{node_id}.internal"),
            os: "Linux".to_string(),
            kernel_version: None,
            cpu_model: None,
            cpu_cores: 2,
            agent_version: "test".to_string(),
            boot_time: None,
            tags: Vec::new(),
        }
    }

    #[test]
    fn token_without_expiry_is_refreshed() {
        let refresh_after =
            Utc::now() + chrono::Duration::days(AGENT_TOKEN_REFRESH_BEFORE_EXPIRY_DAYS);

        assert!(token_needs_refresh(None, refresh_after));
    }

    #[test]
    fn token_expiring_inside_refresh_window_is_refreshed() {
        let refresh_after =
            Utc::now() + chrono::Duration::days(AGENT_TOKEN_REFRESH_BEFORE_EXPIRY_DAYS);

        assert!(token_needs_refresh(
            Some(refresh_after - chrono::Duration::minutes(1)),
            refresh_after,
        ));
    }

    #[test]
    fn token_expiring_after_refresh_window_is_not_refreshed() {
        let refresh_after =
            Utc::now() + chrono::Duration::days(AGENT_TOKEN_REFRESH_BEFORE_EXPIRY_DAYS);

        assert!(!token_needs_refresh(
            Some(refresh_after + chrono::Duration::minutes(1)),
            refresh_after,
        ));
    }

    #[test]
    fn current_token_status_updates_session_cache() {
        let mut session = session();
        let expires_at = Utc::now() + chrono::Duration::days(30);
        let status = RegistryTokenStatus {
            generation: 3,
            token_expires_at: Some(expires_at),
            registry_revision: 12,
        };

        assert!(apply_current_token_status(&mut session, status));
        assert_eq!(session.token_expires_at, Some(expires_at));
        assert_eq!(session.registry_revision, 12);
    }

    #[test]
    fn stale_token_status_does_not_update_session_cache() {
        let mut session = session();
        let status = RegistryTokenStatus {
            generation: 4,
            token_expires_at: Some(Utc::now() + chrono::Duration::days(30)),
            registry_revision: 12,
        };

        assert!(!apply_current_token_status(&mut session, status));
        assert_eq!(session.token_expires_at, None);
        assert_eq!(session.registry_revision, 11);
    }

    #[tokio::test]
    async fn old_token_survives_repeated_refresh_response_send_timeouts() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let temp_dir = std::env::temp_dir().join(format!("nodelite-refresh-timeout-{unique}"));
        std::fs::create_dir_all(&temp_dir).expect("temp dir");
        let path = temp_dir.join("server.json");
        let issued = issue_node(
            &path,
            IssueNodeRequest {
                node_id: "timeout-01".to_string(),
                node_label: Some("Timeout 01".to_string()),
                tags: Vec::new(),
            },
        )
        .await
        .expect("node should be issued");
        let mut registry = NodeRegistry::load(&path)
            .await
            .expect("registry should load");
        let identity = identity_for("timeout-01");
        let mut session_generation = 1;

        for expected_generation in 2..=4 {
            let candidate = registry
                .prepare_token_refresh("timeout-01", session_generation)
                .await
                .expect("token should be persisted before send");
            assert_eq!(candidate.generation, expected_generation);
            let node = registry
                .list_registered_nodes()
                .await
                .into_iter()
                .next()
                .expect("registered node should exist");
            assert!(
                node.previous_token_valid_until.is_none(),
                "unconfirmed sends must not retire the current token"
            );
            assert_eq!(node.token_expires_at, issued.node.token_expires_at);
            let response = nodelite_proto::WireMessage::RefreshTokenResponse(
                nodelite_proto::RefreshTokenResponseMessage {
                    new_token: candidate.token,
                    refresh_id: Some(candidate.refresh_id),
                    expires_at: candidate.expires_at.to_rfc3339(),
                },
            );
            let payload =
                serde_json::to_string(&response).expect("refresh response should serialize");
            let (stream, _blocked_peer) = duplex(1);
            let mut socket = WebSocketStream::from_raw_socket(stream, Role::Server, None).await;

            send_message_with_timeout(
                &mut socket,
                TungsteniteMessage::Text(payload.into()),
                "test refresh response send",
                Duration::from_millis(20),
            )
            .await
            .expect_err("blocked refresh response should time out");
            drop(socket);
            drop(registry);

            registry = NodeRegistry::load(&path)
                .await
                .expect("registry should reload after failed send");
            let authorized = registry
                .authorize(&identity, &issued.node_session_token)
                .await
                .expect("old token should reconnect during persisted grace");
            assert_eq!(authorized.generation, 1);
            session_generation = authorized.generation;
        }

        drop(registry);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(&temp_dir);
    }
}
