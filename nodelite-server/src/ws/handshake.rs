//! 握手、鉴权与会话启动。

use std::net::IpAddr;
use std::time::Duration;

use anyhow::Result;
use axum::extract::ws::WebSocket;
use nodelite_proto::{
    HelloMessage, MIN_SUPPORTED_WIRE_PROTOCOL_VERSION, ServerNoticeMessage, WIRE_PROTOCOL_VERSION,
    WireMessage,
};
use serde_json::json;
use tracing::{info, warn};

use super::ActiveSession;
use super::protocol::{send_close_frame, send_wire_message};
use super::session::run_authenticated_session;
use crate::AppState;
use crate::audit::{AuditEventType, NewAuditEvent};
use crate::registry::{AuthorizedNode, RegistryError};

/// 一次完整的 WebSocket 会话:握手 → 认证 → 数据循环 → 资源回收。
pub(super) async fn handle_socket(
    state: AppState,
    client_ip: IpAddr,
    audit_user_agent: Option<String>,
    _connection_permit: crate::admission::WsConnectionPermit,
    mut socket: WebSocket,
) -> Result<(), super::ProtocolError> {
    let shared = state.shared.clone();
    let hello = wait_for_hello_message(&state, client_ip, &mut socket).await?;
    let authorized =
        authorize_hello(&state, client_ip, &mut socket, &hello, audit_user_agent).await?;
    let AuthorizedNode {
        identity,
        generation,
        token_expires_at,
        registry_revision,
        location_override,
    } = authorized;
    let geoip = state.geoip.lookup(client_ip).await;
    let mut session = ActiveSession {
        node_id: identity.node_id.clone(),
        node_label: identity.node_label.clone(),
        session_id: shared
            .register_node(
                identity,
                Some(client_ip.to_string()),
                geoip,
                location_override,
            )
            .await,
        session_token: hello.token,
        session_generation: generation,
        token_expires_at,
        registry_revision,
    };

    info!(
        node_id = %session.node_id,
        node_label = %session.node_label,
        session_id = session.session_id,
        "node authenticated"
    );

    let session_result = run_authenticated_session(&state, socket, &mut session).await;
    shared
        .mark_disconnected(&session.node_id, session.session_id)
        .await;
    info!(node_id = %session.node_id, session_id = session.session_id, "node disconnected");
    session_result
}

async fn wait_for_hello_message(
    state: &AppState,
    client_ip: IpAddr,
    socket: &mut WebSocket,
) -> Result<HelloMessage, super::ProtocolError> {
    let shared = state.shared.clone();
    let hello_timeout_secs = shared.config().hello_timeout_secs;
    let shutdown = state.shutdown.clone();

    tokio::select! {
        biased;
        _ = shutdown.cancelled() => {
            let _ = send_close_frame(socket, axum::extract::ws::close_code::AWAY, "server shutting down").await;
            Err(super::ProtocolError::Client("server shutting down".to_string()))
        }
        outcome = tokio::time::timeout(
            Duration::from_secs(hello_timeout_secs),
            super::protocol::recv_hello(socket),
        ) => match outcome {
            Ok(Ok(hello)) => Ok(hello),
            Ok(Err(error)) => {
                state.ws_admission.record_auth_failure(client_ip);
                Err(error)
            }
            Err(_) => {
                state.ws_admission.record_auth_failure(client_ip);
                Err(super::ProtocolError::Client(
                    "timed out waiting for hello message".to_string(),
                ))
            }
        },
    }
}

async fn authorize_hello(
    state: &AppState,
    client_ip: IpAddr,
    socket: &mut WebSocket,
    hello: &HelloMessage,
    audit_user_agent: Option<String>,
) -> Result<AuthorizedNode, super::ProtocolError> {
    if let Some(rejection) = protocol_version_rejection(hello.protocol_version) {
        state.ws_admission.record_auth_failure(client_ip);
        let notice = WireMessage::ServerNotice(ServerNoticeMessage {
            level: nodelite_proto::NoticeLevel::Error,
            message: rejection.notice_message,
        });
        let _ = send_wire_message(socket, &notice).await;
        return Err(super::ProtocolError::Client(rejection.error_message));
    }

    match state
        .registry
        .authorize(&hello.identity, &hello.token)
        .await
    {
        Ok(authorized) => {
            state.ws_admission.clear_auth_failures(client_ip);
            let mut event =
                NewAuditEvent::now(AuditEventType::NodeConnected, client_ip.to_string(), true);
            event.node_id = Some(authorized.identity.node_id.clone());
            event.user_agent = audit_user_agent;
            event.details = json!({
                "protocol_version": hello.protocol_version,
            });
            state.audit_log.record_best_effort(event).await;
            Ok(authorized)
        }
        Err(error) => {
            warn!(
                client_ip = %client_ip,
                requested_node_id = %hello.identity.node_id,
                error = ?error,
                "websocket authentication rejected",
            );
            state.ws_admission.record_auth_failure(client_ip);
            let rejection = auth_failure_rejection(&error);
            let notice = WireMessage::ServerNotice(ServerNoticeMessage {
                level: nodelite_proto::NoticeLevel::Error,
                message: rejection.notice_message.to_string(),
            });
            let _ = send_wire_message(socket, &notice).await;
            let mut event =
                NewAuditEvent::now(AuditEventType::TokenInvalid, client_ip.to_string(), false);
            event.node_id = Some(hello.identity.node_id.clone());
            event.user_agent = audit_user_agent;
            event.details = json!({
                "reason": rejection.error_label,
            });
            state.audit_log.record_best_effort(event).await;
            Err(super::ProtocolError::Client(
                rejection.error_label.to_string(),
            ))
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
struct ProtocolVersionRejection {
    notice_message: String,
    error_message: String,
}

fn protocol_version_rejection(protocol_version: u16) -> Option<ProtocolVersionRejection> {
    if (MIN_SUPPORTED_WIRE_PROTOCOL_VERSION..=WIRE_PROTOCOL_VERSION).contains(&protocol_version) {
        return None;
    }

    Some(ProtocolVersionRejection {
        notice_message: format!(
            "unsupported protocol version {}; server supports {}..={}",
            protocol_version, MIN_SUPPORTED_WIRE_PROTOCOL_VERSION, WIRE_PROTOCOL_VERSION
        ),
        error_message: format!(
            "unsupported protocol version {}; supported range {}..={}",
            protocol_version, MIN_SUPPORTED_WIRE_PROTOCOL_VERSION, WIRE_PROTOCOL_VERSION
        ),
    })
}

#[derive(Debug, PartialEq, Eq)]
struct AuthFailureRejection {
    notice_message: &'static str,
    error_label: &'static str,
}

fn auth_failure_rejection(error: &RegistryError) -> AuthFailureRejection {
    match error {
        RegistryError::TokenExpired { node_id } => {
            warn!(expired_node_id = %node_id, "websocket token expired");
            AuthFailureRejection {
                notice_message: "token expired; run `nodelite-server install-agent --rotate-token` and reinstall this node",
                error_label: "token expired",
            }
        }
        RegistryError::Unauthorized => AuthFailureRejection {
            notice_message: "unauthorized",
            error_label: "unauthorized",
        },
        _ => AuthFailureRejection {
            notice_message: "unauthorized",
            error_label: "unauthorized",
        },
    }
}

#[cfg(test)]
mod tests {
    use nodelite_proto::{MIN_SUPPORTED_WIRE_PROTOCOL_VERSION, WIRE_PROTOCOL_VERSION};

    use super::{RegistryError, auth_failure_rejection, protocol_version_rejection};

    #[test]
    fn supported_protocol_versions_are_accepted() {
        assert!(protocol_version_rejection(MIN_SUPPORTED_WIRE_PROTOCOL_VERSION).is_none());
        assert!(protocol_version_rejection(WIRE_PROTOCOL_VERSION).is_none());
    }

    #[test]
    fn unsupported_protocol_versions_get_notice_and_error_messages() {
        let rejection = protocol_version_rejection(MIN_SUPPORTED_WIRE_PROTOCOL_VERSION - 1)
            .expect("old protocol should be rejected");

        assert!(rejection.notice_message.contains("server supports"));
        assert!(rejection.error_message.contains("supported range"));
    }

    #[test]
    fn token_expired_rejection_uses_actionable_notice() {
        let rejection = auth_failure_rejection(&RegistryError::TokenExpired {
            node_id: "hk-01".to_string(),
        });

        assert_eq!(rejection.error_label, "token expired");
        assert!(rejection.notice_message.contains("--rotate-token"));
    }

    #[test]
    fn unauthorized_rejection_does_not_leak_details() {
        let rejection = auth_failure_rejection(&RegistryError::Unauthorized);

        assert_eq!(rejection.error_label, "unauthorized");
        assert_eq!(rejection.notice_message, "unauthorized");
    }

    #[test]
    fn internal_registry_errors_are_reported_as_unauthorized_to_clients() {
        let rejection = auth_failure_rejection(&RegistryError::invalid_config(
            "server.public_base_url",
            "bad scheme",
        ));

        assert_eq!(rejection.error_label, "unauthorized");
        assert_eq!(rejection.notice_message, "unauthorized");
    }
}
