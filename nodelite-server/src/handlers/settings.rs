use std::process::Stdio;
use std::time::Duration;

use axum::extract::{Path as AxumPath, Query, State};
use axum::http::StatusCode;
use axum::response::{AppendHeaders, IntoResponse, Response};
use axum::Json;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncSeekExt, SeekFrom};
use tokio::time::timeout;
use tokio::{fs, process::Command};
use tracing::{error, info};

use crate::AppState;
use crate::auth::{
    ReadonlyRouteAuth, TWO_FACTOR_AUTH_COOKIE, TWO_FACTOR_AUTH_SECS, TWO_FACTOR_PENDING_COOKIE,
    TWO_FACTOR_PENDING_SECS, auth_cookie, constant_time_compare_bytes, decode_totp_secret,
    expire_cookie, secure_cookies, verify_totp_step,
};
use crate::qr::qr_svg_for_text;
use nodelite_proto::{DEFAULT_HISTORY_RETENTION_HOURS, ReadonlyAuthConfig};

const MAX_UPDATE_LOG_CHUNK_BYTES: u64 = 128 * 1024;

#[path = "helpers.rs"]
mod helpers;
use helpers::{
    generate_totp_secret, otpauth_uri, persist_auth_2fa_change, persist_auth_password_change,
    server_build_version, server_update_log_path, server_update_shell_command,
    settings_json_error, validate_password_for_settings,
};

/// 设置页读取的服务端与安全状态。这里刻意不包含任何 token / password 明文。
#[derive(Debug, Serialize)]
pub(crate) struct SettingsResponse {
    service: &'static str,
    server_version: &'static str,
    repository: &'static str,
    public_base_url: String,
    listen: String,
    config_path: String,
    registry_path: String,
    history_db_path: String,
    snapshot_path: String,
    history_retention_hours: u64,
    refresh_interval_secs: u64,
    auth: SettingsAuth,
    updates: SettingsUpdates,
    agents: Vec<SettingsAgentToken>,
}

#[derive(Debug, Serialize)]
pub(crate) struct SettingsAuth {
    enabled: bool,
    username: Option<String>,
    two_factor_enabled: bool,
    totp_secret_configured: bool,
    session_ttl_secs: u64,
    pending_ttl_secs: u64,
}

#[derive(Debug, Serialize)]
pub(crate) struct SettingsUpdates {
    latest_release_url: String,
    server_upgrade_command: String,
    agent_upgrade_command: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct SettingsAgentToken {
    node_id: String,
    node_label: String,
    online: bool,
    agent_version: Option<String>,
    remote_ip: Option<String>,
    tags: Vec<String>,
    token_expires_at: Option<DateTime<Utc>>,
    token_expires_in_secs: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ChangePasswordRequest {
    current_password: String,
    new_password: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct StartServerUpdateRequest {
    current_password: Option<String>,
    code: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ServerUpdateLogQuery {
    offset: Option<u64>,
}

#[derive(Debug, Serialize)]
pub(crate) struct ServerUpdateLogResponse {
    exists: bool,
    offset: u64,
    next_offset: u64,
    text: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct TwoFactorSetupResponse {
    secret: String,
    otpauth_uri: String,
    qr_svg: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct EnableTwoFactorRequest {
    current_password: String,
    secret: String,
    code: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct DisableTwoFactorRequest {
    current_password: String,
    code: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct SettingsActionResponse {
    ok: bool,
    message: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct NodeTokenRefreshResponse {
    ok: bool,
    message: String,
    token_expires_at: Option<DateTime<Utc>>,
    token_expires_in_secs: Option<i64>,
}

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

/// 修改只读面板密码:需要当前密码,同时更新运行时鉴权与 server.toml。
pub(crate) async fn change_readonly_password(
    State(state): State<AppState>,
    Json(request): Json<ChangePasswordRequest>,
) -> Response {
    let current_auth = {
        let auth = state.readonly_auth.read().await;
        auth.config.clone()
    };
    let Some(current_auth) = current_auth else {
        return settings_json_error(StatusCode::CONFLICT, "readonly auth is not enabled");
    };
    if !constant_time_compare_bytes(
        current_auth.password.as_bytes(),
        request.current_password.as_bytes(),
    ) {
        return settings_json_error(StatusCode::UNAUTHORIZED, "current password is incorrect");
    }
    if let Err(message) = validate_password_for_settings(&request.new_password) {
        return settings_json_error(StatusCode::BAD_REQUEST, message);
    }

    let next_auth = ReadonlyAuthConfig {
        password: request.new_password.clone(),
        ..current_auth
    };
    let config_path = state.config_path.as_path().to_path_buf();
    if let Err(error) = persist_auth_password_change(&config_path, &next_auth.password).await {
        error!(error = ?error, path = %config_path.display(), "failed to persist readonly password change");
        return settings_json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "failed to persist password change",
        );
    }
    {
        let mut auth = state.readonly_auth.write().await;
        *auth = ReadonlyRouteAuth::from_config(Some(next_auth));
    }
    state.two_factor_sessions.clear_authenticated();
    let secure = secure_cookies(state.shared.config());
    (
        StatusCode::OK,
        AppendHeaders([
            expire_cookie(TWO_FACTOR_AUTH_COOKIE, secure),
            expire_cookie(TWO_FACTOR_PENDING_COOKIE, secure),
        ]),
        Json(SettingsActionResponse {
            ok: true,
            message: "password changed; please sign in again".to_string(),
        }),
    )
        .into_response()
}

/// 从网页端手动触发一次服务端升级。
///
/// 这是一个显式、实验性的运维入口:若开启 2FA,调用方必须输入当前 TOTP;
/// 否则回退到当前只读密码确认。通过确认后,服务端在后台拉取 GitHub latest
/// release 里的安装脚本并以 upgrade 模式运行。这里不等待子进程结束,因为升级
/// 过程可能会重启当前服务。
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
    let command = server_update_shell_command(&log_path);
    let unit_name = format!(
        "nodelite-server-web-update-{}",
        Utc::now().timestamp_millis()
    );
    match Command::new("systemd-run")
        .arg(format!("--unit={unit_name}"))
        .arg("--collect")
        .arg("--service-type=exec")
        .arg("--property=ProtectSystem=no")
        .arg("--property=ProtectHome=no")
        .arg("--property=PrivateTmp=no")
        .arg("--property=NoNewPrivileges=no")
        .arg("sh")
        .arg("-c")
        .arg(command)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(_) => {
            info!(unit = %unit_name, "manual server update started from settings page");
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
///
/// 这条路径与服务端的自动续期使用同一套"刷新注册表 → 推送新 token 给 Agent →
/// Agent 持久化到本地 agent.toml"逻辑。只有在线节点才能执行,离线节点不会落成
/// "静默改 registry 导致当前 Agent 下次上线被拒绝"的危险状态。
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

    let refresh_result = match timeout(Duration::from_secs(20), refresh_receiver).await {
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
            return settings_json_error(StatusCode::BAD_GATEWAY, &message);
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

fn settings_confirmation_error_for_sensitive_action(
    state: &AppState,
    auth: &ReadonlyAuthConfig,
    current_password: Option<&str>,
    code: Option<&str>,
) -> Option<Response> {
    if !auth.enable_2fa {
        let Some(current_password) = current_password.filter(|password| !password.is_empty())
        else {
            return Some(settings_json_error(
                StatusCode::UNAUTHORIZED,
                "current password is required",
            ));
        };
        if constant_time_compare_bytes(auth.password.as_bytes(), current_password.as_bytes()) {
            return None;
        }
        return Some(settings_json_error(
            StatusCode::UNAUTHORIZED,
            "current password is incorrect",
        ));
    }
    let Some(secret) = auth.totp_secret.as_deref().and_then(decode_totp_secret) else {
        return Some(settings_json_error(
            StatusCode::CONFLICT,
            "2FA secret is not configured",
        ));
    };
    let Some(code) = code.map(str::trim).filter(|code| !code.is_empty()) else {
        return Some(settings_json_error(
            StatusCode::UNAUTHORIZED,
            "verification code is required",
        ));
    };
    let Some(step) = verify_totp_step(Some(&secret), code) else {
        return Some(settings_json_error(
            StatusCode::UNAUTHORIZED,
            "invalid verification code",
        ));
    };
    if state.two_factor_sessions.is_totp_step_used(step) {
        return Some(settings_json_error(
            StatusCode::UNAUTHORIZED,
            "verification code already used",
        ));
    }
    state.two_factor_sessions.mark_totp_step_used(step);
    None
}

/// 开始网页端 2FA 绑定:生成一个新 TOTP secret 和本地 SVG 二维码。
///
/// 注意这里不写配置文件;只有用户用认证器 App 扫码并输入正确验证码后,
/// `enable_two_factor` 才会真正启用 2FA。
pub(crate) async fn start_two_factor_setup(State(state): State<AppState>) -> Response {
    let current_auth = {
        let auth = state.readonly_auth.read().await;
        auth.config.clone()
    };
    let Some(current_auth) = current_auth else {
        return settings_json_error(StatusCode::CONFLICT, "readonly auth is not enabled");
    };
    if !state
        .shared
        .config()
        .public_base_url
        .starts_with("https://")
    {
        return settings_json_error(
            StatusCode::CONFLICT,
            "2FA setup requires server.public_base_url to use https://",
        );
    }

    let secret = match generate_totp_secret() {
        Ok(secret) => secret,
        Err(error) => {
            error!(error = ?error, "failed to generate TOTP secret");
            return settings_json_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "failed to generate TOTP secret",
            );
        }
    };
    let otpauth_uri = otpauth_uri(&current_auth.username, &secret);
    let qr_svg = match qr_svg_for_text(&otpauth_uri) {
        Ok(svg) => svg,
        Err(error) => {
            error!(error = ?error, "failed to render TOTP QR code");
            return settings_json_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "failed to render TOTP QR code",
            );
        }
    };

    Json(TwoFactorSetupResponse {
        secret,
        otpauth_uri,
        qr_svg,
    })
    .into_response()
}

/// 启用 2FA:要求当前密码 + 新 secret 对应的 6 位 TOTP 验证码。
pub(crate) async fn enable_two_factor(
    State(state): State<AppState>,
    Json(request): Json<EnableTwoFactorRequest>,
) -> Response {
    let current_auth = {
        let auth = state.readonly_auth.read().await;
        auth.config.clone()
    };
    let Some(current_auth) = current_auth else {
        return settings_json_error(StatusCode::CONFLICT, "readonly auth is not enabled");
    };
    if !constant_time_compare_bytes(
        current_auth.password.as_bytes(),
        request.current_password.as_bytes(),
    ) {
        return settings_json_error(StatusCode::UNAUTHORIZED, "current password is incorrect");
    }
    let secret = request.secret.replace(' ', "").to_ascii_uppercase();
    let Some(secret_bytes) = decode_totp_secret(&secret) else {
        return settings_json_error(StatusCode::BAD_REQUEST, "invalid TOTP secret");
    };
    if secret_bytes.len() < 10 {
        return settings_json_error(StatusCode::BAD_REQUEST, "invalid TOTP secret");
    }
    let Some(step) = verify_totp_step(Some(&secret_bytes), &request.code) else {
        return settings_json_error(StatusCode::UNAUTHORIZED, "invalid verification code");
    };
    if state.two_factor_sessions.is_totp_step_used(step) {
        return settings_json_error(StatusCode::UNAUTHORIZED, "verification code already used");
    }
    state.two_factor_sessions.mark_totp_step_used(step);

    let next_auth = ReadonlyAuthConfig {
        enable_2fa: true,
        totp_secret: Some(secret),
        ..current_auth
    };
    if let Err(error) = persist_auth_2fa_change(state.config_path.as_path(), &next_auth).await {
        error!(error = ?error, path = %state.config_path.display(), "failed to persist 2FA enable");
        return settings_json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "failed to persist 2FA settings",
        );
    }
    {
        let mut auth = state.readonly_auth.write().await;
        *auth = ReadonlyRouteAuth::from_config(Some(next_auth));
    }
    state.two_factor_sessions.clear_authenticated();
    let auth_token = match state.two_factor_sessions.create_authenticated() {
        Ok(token) => token,
        Err(error) => {
            error!(error = ?error, "failed to create 2FA session after enabling 2FA");
            return settings_json_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "failed to create authenticated session",
            );
        }
    };
    let secure = secure_cookies(state.shared.config());
    (
        StatusCode::OK,
        AppendHeaders([
            auth_cookie(
                TWO_FACTOR_AUTH_COOKIE,
                &auth_token,
                TWO_FACTOR_AUTH_SECS,
                secure,
            ),
            expire_cookie(TWO_FACTOR_PENDING_COOKIE, secure),
        ]),
        Json(SettingsActionResponse {
            ok: true,
            message: "2FA enabled".to_string(),
        }),
    )
        .into_response()
}

/// 关闭 2FA:要求当前密码 + 当前 TOTP 验证码,避免无人值守浏览器被直接降级。
pub(crate) async fn disable_two_factor(
    State(state): State<AppState>,
    Json(request): Json<DisableTwoFactorRequest>,
) -> Response {
    let current_auth = {
        let auth = state.readonly_auth.read().await;
        auth.config.clone()
    };
    let Some(current_auth) = current_auth else {
        return settings_json_error(StatusCode::CONFLICT, "readonly auth is not enabled");
    };
    if !current_auth.enable_2fa {
        return settings_json_error(StatusCode::CONFLICT, "2FA is not enabled");
    }
    if !constant_time_compare_bytes(
        current_auth.password.as_bytes(),
        request.current_password.as_bytes(),
    ) {
        return settings_json_error(StatusCode::UNAUTHORIZED, "current password is incorrect");
    }
    let Some(secret) = current_auth
        .totp_secret
        .as_deref()
        .and_then(decode_totp_secret)
    else {
        return settings_json_error(StatusCode::CONFLICT, "2FA secret is not configured");
    };
    let Some(step) = verify_totp_step(Some(&secret), &request.code) else {
        return settings_json_error(StatusCode::UNAUTHORIZED, "invalid verification code");
    };
    if state.two_factor_sessions.is_totp_step_used(step) {
        return settings_json_error(StatusCode::UNAUTHORIZED, "verification code already used");
    }
    state.two_factor_sessions.mark_totp_step_used(step);

    let next_auth = ReadonlyAuthConfig {
        enable_2fa: false,
        totp_secret: None,
        ..current_auth
    };
    if let Err(error) = persist_auth_2fa_change(state.config_path.as_path(), &next_auth).await {
        error!(error = ?error, path = %state.config_path.display(), "failed to persist 2FA disable");
        return settings_json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "failed to persist 2FA settings",
        );
    }
    {
        let mut auth = state.readonly_auth.write().await;
        *auth = ReadonlyRouteAuth::from_config(Some(next_auth));
    }
    state.two_factor_sessions.clear_authenticated();
    let secure = secure_cookies(state.shared.config());
    (
        StatusCode::OK,
        AppendHeaders([
            expire_cookie(TWO_FACTOR_AUTH_COOKIE, secure),
            expire_cookie(TWO_FACTOR_PENDING_COOKIE, secure),
        ]),
        Json(SettingsActionResponse {
            ok: true,
            message: "2FA disabled; please sign in again".to_string(),
        }),
    )
        .into_response()
}
