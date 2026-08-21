//! 网页端 Agent 注销：安全移除注册表条目并立即从运行态视图撤下节点。

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use tracing::{error, info};

use crate::AppState;
use crate::registry::RegistryError;
use crate::snapshot::persist_snapshot;

use super::security::settings_confirmation_error_for_sensitive_action;
use super::{DeleteAgentRequest, SettingsActionResponse, settings_json_error};

/// 注销一个 Agent，撤销其凭证以及待消费的安装令牌。
pub(crate) async fn delete_agent(
    State(state): State<AppState>,
    Path(node_id): Path<String>,
    Json(request): Json<DeleteAgentRequest>,
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

    let removed = match state.registry.remove_node(&node_id).await {
        Ok(node) => node,
        Err(RegistryError::NodeNotFound(_)) => {
            return settings_json_error(StatusCode::NOT_FOUND, "node not found");
        }
        Err(RegistryError::Validation { .. }) => {
            return settings_json_error(StatusCode::BAD_REQUEST, "invalid node id");
        }
        Err(error) => {
            error!(error = ?error, node_id = %node_id, "failed to remove agent from registry");
            return settings_json_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "failed to remove agent",
            );
        }
    };

    state.shared.remove_node(&removed.node_id).await;
    let remaining_statuses = state.shared.list_statuses().await;
    if let Err(error) = persist_snapshot(
        state.shared.config().snapshot_path.as_path(),
        &remaining_statuses,
    )
    .await
    {
        error!(
            error = ?error,
            node_id = %removed.node_id,
            "failed to persist snapshot after removing agent"
        );
    }
    info!(node_id = %removed.node_id, "agent removed from settings page");

    (
        StatusCode::OK,
        Json(SettingsActionResponse {
            ok: true,
            message: "agent removed".to_string(),
        }),
    )
        .into_response()
}
