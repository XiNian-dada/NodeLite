//! 网页端 Agent 安装命令签发:复用 CLI 的注册表写入和命令渲染流程。

use axum::Json;
use axum::extract::State;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use tracing::error;

use crate::AppState;
use crate::registry::{
    IssueNodeRequest, RegistryError, default_agent_release_base_url, issue_node,
    render_install_command,
};

use super::security::settings_confirmation_error_for_sensitive_action;
use super::{GenerateAgentInstallRequest, GenerateAgentInstallResponse, settings_json_error};

/// 签发一次性安装令牌并返回可直接在目标主机运行的 Agent 安装命令。
pub(crate) async fn generate_agent_install(
    State(state): State<AppState>,
    Json(request): Json<GenerateAgentInstallRequest>,
) -> Response {
    let current_auth = {
        let auth = state.readonly_auth.read().await;
        auth.config.clone()
    };
    let Some(current_auth) = current_auth else {
        return settings_json_error(StatusCode::CONFLICT, "readonly auth is not enabled");
    };
    if let Some(response) = settings_confirmation_error_for_sensitive_action(
        &state,
        &current_auth,
        request.current_password.as_deref(),
        request.code.as_deref(),
    ) {
        return response;
    }

    let issued = match issue_node(
        state.registry.path(),
        IssueNodeRequest {
            node_id: request.node_id,
            node_label: request.node_label,
            tags: request.tags,
        },
    )
    .await
    {
        Ok(issued) => issued,
        Err(RegistryError::Validation { message }) => {
            return settings_json_error(StatusCode::BAD_REQUEST, message);
        }
        Err(error) => {
            error!(error = ?error, "failed to issue agent installation command");
            return settings_json_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "failed to generate agent installation command",
            );
        }
    };
    let agent_release_base_url = match default_agent_release_base_url() {
        Ok(url) => url,
        Err(error) => {
            error!(error = ?error, "failed to resolve the default agent release URL");
            return settings_json_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "failed to generate agent installation command",
            );
        }
    };
    let install_command = match render_install_command(
        &state.shared.config().public_base_url,
        &issued.install_token,
        &agent_release_base_url,
    ) {
        Ok(command) => command,
        Err(error) => {
            error!(error = ?error, "failed to render agent installation command");
            return settings_json_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "failed to generate agent installation command",
            );
        }
    };
    if let Err(error) = state.registry.reload().await {
        error!(error = ?error, "failed to reload registry after issuing agent install command");
        return settings_json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "failed to refresh the node registry",
        );
    }

    let message = if issued.created {
        "agent install command generated"
    } else {
        "agent token rotated and install command generated"
    };
    (
        StatusCode::OK,
        [(header::CACHE_CONTROL, "no-store")],
        Json(GenerateAgentInstallResponse {
            ok: true,
            message: message.to_string(),
            node_id: issued.node.node_id,
            node_label: issued.node.node_label,
            created: issued.created,
            install_token_expires_at: issued.install_token_expires_at,
            install_command,
        }),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::time::{SystemTime, UNIX_EPOCH};

    use axum::Json;
    use axum::body::to_bytes;
    use axum::extract::State;
    use axum::http::{StatusCode, header};
    use serde_json::Value;

    use super::generate_agent_install;
    use crate::AppState;
    use crate::handlers::settings::GenerateAgentInstallRequest;
    use crate::test_support::test_server_config;

    fn unique_temp_dir(prefix: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after the Unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("{prefix}-{unique}"))
    }

    async fn test_state() -> (AppState, PathBuf) {
        let temp_dir = unique_temp_dir("nodelite-agent-install-handler-test");
        std::fs::create_dir_all(&temp_dir).expect("test temp dir should be created");
        let config = test_server_config(
            SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 8080)),
            "https://monitor.example.com".to_string(),
            temp_dir.join("registry.json"),
            temp_dir.join("history.sqlite3"),
            temp_dir.join("snapshot.json"),
        );
        let state =
            AppState::test_fixture(Arc::new(config), Arc::new(temp_dir.join("server.toml")))
                .await
                .expect("test state should be created");
        (state, temp_dir)
    }

    async fn cleanup(state: AppState, temp_dir: PathBuf) {
        state.shutdown.cancel();
        state.history.shutdown().await;
        state.audit_log.shutdown().await;
        drop(state);
        std::fs::remove_dir_all(temp_dir).expect("test temp dir should be removed");
    }

    #[tokio::test]
    async fn issues_a_node_and_returns_a_non_cacheable_install_command() {
        let (state, temp_dir) = test_state().await;
        let response = generate_agent_install(
            State(state.clone()),
            Json(GenerateAgentInstallRequest {
                node_id: "sg-01".to_string(),
                node_label: Some("Singapore 01".to_string()),
                tags: vec!["edge".to_string(), "prod".to_string()],
                current_password: Some("secret".to_string()),
                code: None,
            }),
        )
        .await;

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers().get(header::CACHE_CONTROL),
            Some(&header::HeaderValue::from_static("no-store")),
        );
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("response body should collect");
        let payload: Value = serde_json::from_slice(&body).expect("response body should be json");
        assert_eq!(payload["node_id"], "sg-01");
        assert_eq!(payload["node_label"], "Singapore 01");
        assert_eq!(payload["created"], true);
        assert!(
            payload["install_command"]
                .as_str()
                .is_some_and(|command| command.contains("NODELITE_AGENT_INSTALL_TOKEN="))
        );

        let nodes = state.registry.list_registered_nodes().await;
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].tags, vec!["edge", "prod"]);
        cleanup(state, temp_dir).await;
    }

    #[tokio::test]
    async fn rejects_agent_install_without_sensitive_confirmation() {
        let (state, temp_dir) = test_state().await;
        let response = generate_agent_install(
            State(state.clone()),
            Json(GenerateAgentInstallRequest {
                node_id: "sg-01".to_string(),
                node_label: None,
                tags: Vec::new(),
                current_password: Some("wrong".to_string()),
                code: None,
            }),
        )
        .await;

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert!(state.registry.list_registered_nodes().await.is_empty());
        cleanup(state, temp_dir).await;
    }

    #[tokio::test]
    async fn rejects_an_invalid_node_id_without_mutating_the_registry() {
        let (state, temp_dir) = test_state().await;
        let response = generate_agent_install(
            State(state.clone()),
            Json(GenerateAgentInstallRequest {
                node_id: "invalid node id".to_string(),
                node_label: None,
                tags: Vec::new(),
                current_password: Some("secret".to_string()),
                code: None,
            }),
        )
        .await;

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert!(state.registry.list_registered_nodes().await.is_empty());
        cleanup(state, temp_dir).await;
    }
}
