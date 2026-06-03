use std::time::Duration;

use axum::Json;
use axum::extract::{Path as AxumPath, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use chrono::Utc;
use tokio::fs;
use tokio::io::{AsyncReadExt, AsyncSeekExt, SeekFrom};
use tokio::time::timeout;
use tracing::{error, info};

use crate::AppState;

use super::security::settings_confirmation_error_for_sensitive_action;
use super::{
    MAX_UPDATE_LOG_CHUNK_BYTES, NodeTokenRefreshResponse, ServerUpdateLogQuery,
    ServerUpdateLogResponse, SettingsActionResponse, StartServerUpdateRequest, UpdateLaunchMode,
    is_writable_paths_subset_of_install_root, server_update_cache_dir, server_update_install_root,
    server_update_log_path, server_update_shell_command, server_update_writable_paths,
    settings_json_error, spawn_server_update_subprocess,
};

#[cfg(not(test))]
const LIVE_TOKEN_REFRESH_TIMEOUT: Duration = Duration::from_secs(20);
#[cfg(test)]
const LIVE_TOKEN_REFRESH_TIMEOUT: Duration = Duration::from_millis(50);

/// 从网页端手动触发一次服务端升级。
pub(crate) async fn start_server_update(
    State(state): State<AppState>,
    Json(request): Json<StartServerUpdateRequest>,
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

    let log_path = server_update_log_path(state.shared.config());
    let cache_dir = server_update_cache_dir(state.shared.config());
    let command = server_update_shell_command(&log_path, &cache_dir);
    let writable_paths = server_update_writable_paths(state.shared.config());
    let install_root = server_update_install_root(state.shared.config());
    if !is_writable_paths_subset_of_install_root(&writable_paths, &install_root) {
        error!(
            install_root = %install_root.display(),
            paths = ?writable_paths,
            "server update writable paths escaped the allowed roots"
        );
        return settings_json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "failed to start server update",
        );
    }
    let unit_name = format!(
        "nodelite-server-web-update-{}",
        Utc::now().timestamp_millis()
    );
    match spawn_server_update_subprocess(&unit_name, &command, &writable_paths).await {
        Ok(launch_mode) => {
            let launch_mode = match launch_mode {
                UpdateLaunchMode::Systemd => "systemd",
                UpdateLaunchMode::FallbackShell => "shell-fallback",
            };
            info!(
                unit = %unit_name,
                launch_mode,
                "manual server update started from settings page"
            );
            (
                StatusCode::ACCEPTED,
                Json(SettingsActionResponse {
                    ok: true,
                    message: "server update started; the service may restart shortly".to_string(),
                }),
            )
                .into_response()
        }
        Err(error) => {
            error!(error = ?error, "failed to start manual server update");
            settings_json_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "failed to start server update",
            )
        }
    }
}

/// 节点级敏感操作:通过当前在线 WebSocket 会话立即续期指定 Agent 的 token。
pub(crate) async fn refresh_node_token(
    State(state): State<AppState>,
    AxumPath(node_id): AxumPath<String>,
    Json(request): Json<StartServerUpdateRequest>,
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

    let Some(node) = state.shared.get_status(&node_id).await else {
        return settings_json_error(StatusCode::NOT_FOUND, "node not found");
    };
    if !node.online {
        return settings_json_error(
            StatusCode::CONFLICT,
            "node is offline; online refresh requires an active agent session",
        );
    }

    let refresh_receiver = match state.shared.request_live_token_refresh(&node_id).await {
        Ok(receiver) => receiver,
        Err(error) => {
            return settings_json_error(StatusCode::CONFLICT, error.to_string());
        }
    };

    let refresh_result = match timeout(LIVE_TOKEN_REFRESH_TIMEOUT, refresh_receiver).await {
        Ok(Ok(result)) => result,
        Ok(Err(_)) => {
            return settings_json_error(
                StatusCode::CONFLICT,
                "agent session closed before refresh completed",
            );
        }
        Err(_) => {
            return settings_json_error(
                StatusCode::GATEWAY_TIMEOUT,
                "timed out waiting for agent token refresh",
            );
        }
    };

    let token_expires_at = match refresh_result {
        Ok(reply) => Some(reply.token_expires_at),
        Err(message) => {
            error!(node_id = %node_id, error = %message, "manual online token refresh failed");
            return settings_json_error(
                StatusCode::BAD_GATEWAY,
                "failed to refresh token on the online agent",
            );
        }
    };
    let now = Utc::now();

    (
        StatusCode::OK,
        Json(NodeTokenRefreshResponse {
            ok: true,
            message: "agent token refreshed and pushed to the online node".to_string(),
            token_expires_at,
            token_expires_in_secs: token_expires_at
                .map(|expires_at| (expires_at - now).num_seconds()),
        }),
    )
        .into_response()
}

pub(crate) async fn server_update_log(
    State(state): State<AppState>,
    Query(query): Query<ServerUpdateLogQuery>,
) -> Response {
    let log_path = server_update_log_path(state.shared.config());
    let metadata = match fs::metadata(&log_path).await {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Json(ServerUpdateLogResponse {
                exists: false,
                offset: 0,
                next_offset: 0,
                text: String::new(),
            })
            .into_response();
        }
        Err(error) => {
            error!(error = ?error, path = %log_path.display(), "failed to inspect update log");
            return settings_json_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "failed to inspect update log",
            );
        }
    };

    let file_len = metadata.len();
    let requested_offset = query.offset.unwrap_or(0).min(file_len);
    let capped_offset = requested_offset.max(file_len.saturating_sub(MAX_UPDATE_LOG_CHUNK_BYTES));
    let mut file = match fs::File::open(&log_path).await {
        Ok(file) => file,
        Err(error) => {
            error!(error = ?error, path = %log_path.display(), "failed to open update log");
            return settings_json_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "failed to open update log",
            );
        }
    };
    if let Err(error) = file.seek(SeekFrom::Start(capped_offset)).await {
        error!(error = ?error, path = %log_path.display(), "failed to seek update log");
        return settings_json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "failed to read update log",
        );
    }
    let mut bytes = Vec::new();
    if let Err(error) = file.read_to_end(&mut bytes).await {
        error!(error = ?error, path = %log_path.display(), "failed to read update log");
        return settings_json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "failed to read update log",
        );
    }
    let next_offset = capped_offset.saturating_add(bytes.len() as u64);
    Json(ServerUpdateLogResponse {
        exists: true,
        offset: capped_offset,
        next_offset,
        text: String::from_utf8_lossy(&bytes).into_owned(),
    })
    .into_response()
}
