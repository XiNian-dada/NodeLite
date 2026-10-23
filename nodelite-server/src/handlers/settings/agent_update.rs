//! Step-up-protected signals, never remote installation commands or download locations.

use std::net::SocketAddr;

use axum::Json;
use axum::extract::{ConnectInfo, Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use nodelite_proto::agent_update::{AgentUpdateRequest, AgentUpdateStatus, REQUEST_LIFETIME_SECS};
use serde::{Deserialize, Serialize};

use crate::AppState;
use crate::admission::resolve_client_ip;
use crate::agent_updates::RequestError;
use crate::audit::{AuditEventType, NewAuditEvent};

use super::security::settings_confirmation_error_for_sensitive_action;
use super::settings_json_error;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StartAgentUpdate {
    current_password: Option<String>,
    code: Option<String>,
}

#[derive(Serialize)]
struct AgentUpdateView {
    online: bool,
    agent_version: String,
    can_update: bool,
    status: AgentUpdateStatus,
}

fn now() -> u64 {
    chrono::Utc::now().timestamp().max(0) as u64
}

pub(crate) async fn agent_update(
    State(state): State<AppState>,
    Path(node_id): Path<String>,
) -> Response {
    let Some(node) = state.shared.get_status(&node_id).await else {
        return settings_json_error(StatusCode::NOT_FOUND, "node not found");
    };
    let (status, can_update) = state.agent_updates.view(&node_id, now());
    Json(AgentUpdateView {
        online: node.online,
        agent_version: node.identity.agent_version,
        can_update: can_update && node.online,
        status,
    })
    .into_response()
}

pub(crate) async fn start_agent_update(
    State(state): State<AppState>,
    Path(node_id): Path<String>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(request): Json<StartAgentUpdate>,
) -> Response {
    let Some(auth) = state.readonly_auth.read().await.config.clone() else {
        return settings_json_error(StatusCode::CONFLICT, "readonly auth is not enabled");
    };
    if let Some(response) = settings_confirmation_error_for_sensitive_action(
        &state,
        &auth,
        &headers,
        request.current_password.as_deref(),
        request.code.as_deref(),
    ) {
        return response;
    }
    let Some(node) = state.shared.get_status(&node_id).await else {
        return settings_json_error(StatusCode::NOT_FOUND, "node not found");
    };
    if !node.online {
        return settings_json_error(StatusCode::CONFLICT, "node is offline");
    }
    queue_update(state, node_id, peer, headers).await
}

async fn queue_update(
    state: AppState,
    node_id: String,
    peer: SocketAddr,
    headers: HeaderMap,
) -> Response {
    let mut nonce = [0u8; 16];
    if getrandom::fill(&mut nonce).is_err() {
        return settings_json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "cannot allocate update request",
        );
    }
    let issued_at = now();
    let request = AgentUpdateRequest {
        request_id: nonce.iter().map(|byte| format!("{byte:02x}")).collect(),
        issued_at,
        expires_at: issued_at + REQUEST_LIFETIME_SECS,
    };
    if let Err(error) = state.agent_updates.reserve(&node_id, request.clone()) {
        let status = if matches!(error, RequestError::RateLimited) {
            StatusCode::TOO_MANY_REQUESTS
        } else {
            StatusCode::CONFLICT
        };
        return settings_json_error(status, error.to_string());
    }
    if let Err(error) = state
        .shared
        .request_agent_update(&node_id, request.clone())
        .await
    {
        state
            .agent_updates
            .delivery_failed(&node_id, &request.request_id);
        return settings_json_error(StatusCode::CONFLICT, error.to_string());
    }
    let ip = resolve_client_ip(&state.shared.config().trusted_proxies, peer, &headers).to_string();
    let mut event = NewAuditEvent::now(AuditEventType::AgentUpdateRequested, ip, true);
    event.node_id = Some(node_id.clone());
    event.details = serde_json::json!({"request_id": request.request_id, "phase": "queued"});
    state.audit_log.record_best_effort(event).await;
    (
        StatusCode::ACCEPTED,
        agent_update(State(state), Path(node_id)).await,
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn step_up_precedes_lookup_and_rejects_an_incorrect_password() {
        let (state, directory) =
            crate::tests::two_factor_auth_test_state("agent-web-update", false).await;
        let peer: SocketAddr = "127.0.0.1:51234".parse().expect("peer");
        for (password, expected) in [
            (None, StatusCode::PRECONDITION_REQUIRED),
            (Some("incorrect"), StatusCode::FORBIDDEN),
            (Some("secret"), StatusCode::NOT_FOUND),
        ] {
            let result = start_agent_update(
                State(state.clone()),
                Path("missing".into()),
                ConnectInfo(peer),
                HeaderMap::new(),
                Json(StartAgentUpdate {
                    current_password: password.map(str::to_string),
                    code: None,
                }),
            )
            .await;
            assert_eq!(result.status(), expected);
        }
        state.shutdown.cancel();
        state.history.shutdown().await;
        state.audit_log.shutdown().await;
        drop(state);
        std::fs::remove_dir_all(directory).expect("remove test-owned directory");
    }

    #[test]
    fn web_requests_cannot_supply_commands_channels_or_download_urls() {
        for field in ["command", "url", "channel", "service", "path", "version"] {
            let value = serde_json::json!({(field): "arbitrary"});
            assert!(
                serde_json::from_value::<StartAgentUpdate>(value).is_err(),
                "{field}"
            );
        }
        assert!(serde_json::from_str::<StartAgentUpdate>("{}").is_ok());
    }
}
