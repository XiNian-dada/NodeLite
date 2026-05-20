use axum::Json;
use axum::extract::State;
use axum::response::IntoResponse;
use chrono::Utc;

use crate::AppState;
use crate::auth::{TWO_FACTOR_AUTH_SECS, TWO_FACTOR_PENDING_SECS};
use nodelite_proto::DEFAULT_HISTORY_RETENTION_HOURS;

use super::{
    SettingsAgentToken, SettingsAuth, SettingsResponse, SettingsUpdates, server_build_version,
};

/// 设置页数据接口:只返回运行状态与安全元信息,不泄露任何凭证本体。
pub(crate) async fn settings(State(state): State<AppState>) -> impl IntoResponse {
    let config = state.shared.config();
    let runtime_auth = {
        let auth = state.readonly_auth.read().await;
        auth.config.clone()
    };
    let statuses = state.shared.list_statuses().await;
    let status_by_id = statuses
        .into_iter()
        .map(|status| (status.identity.node_id.clone(), status))
        .collect::<std::collections::HashMap<_, _>>();
    let now = Utc::now();
    let agents = state
        .registry
        .list_registered_nodes()
        .await
        .into_iter()
        .map(|node| {
            let status = status_by_id.get(&node.node_id);
            SettingsAgentToken {
                node_id: node.node_id,
                node_label: node.node_label,
                online: status.is_some_and(|status| status.online),
                agent_version: status.map(|status| status.identity.agent_version.clone()),
                remote_ip: status.and_then(|status| status.remote_ip.clone()),
                tags: node.tags,
                token_expires_at: node.token_expires_at,
                token_expires_in_secs: node
                    .token_expires_at
                    .map(|expires_at| (expires_at - now).num_seconds()),
            }
        })
        .collect();
    let auth = runtime_auth.as_ref();
    Json(SettingsResponse {
        service: "nodelite-server",
        server_version: server_build_version(),
        repository: env!("CARGO_PKG_REPOSITORY"),
        public_base_url: config.public_base_url.clone(),
        listen: config.listen.to_string(),
        config_path: state.config_path.display().to_string(),
        registry_path: config.node_registry_path.display().to_string(),
        history_db_path: config.history_db_path.display().to_string(),
        snapshot_path: config.snapshot_path.display().to_string(),
        history_retention_hours: DEFAULT_HISTORY_RETENTION_HOURS,
        refresh_interval_secs: config.refresh_interval_secs,
        auth: SettingsAuth {
            enabled: auth.is_some(),
            username: auth.map(|auth| auth.username.clone()),
            two_factor_enabled: auth.is_some_and(|auth| auth.enable_2fa),
            totp_secret_configured: auth.and_then(|auth| auth.totp_secret.as_ref()).is_some(),
            session_ttl_secs: TWO_FACTOR_AUTH_SECS,
            pending_ttl_secs: TWO_FACTOR_PENDING_SECS,
        },
        updates: SettingsUpdates {
            latest_release_url: format!("{}/releases/latest", env!("CARGO_PKG_REPOSITORY")),
            server_upgrade_command: "curl -fsSL https://github.com/XiNian-dada/NodeLite/releases/latest/download/install-server.sh | sudo NODELITE_SERVER_MODE=upgrade sh".to_string(),
            agent_upgrade_command: format!(
                "{} upgrade-agent",
                std::env::args()
                    .next()
                    .unwrap_or_else(|| "nodelite-server".to_string())
            ),
        },
        agents,
    })
}
