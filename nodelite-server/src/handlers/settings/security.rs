use axum::Json;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{AppendHeaders, IntoResponse, Response};
use tracing::error;

use crate::AppState;
use crate::auth::{
    BASIC_AUTH_SESSION_COOKIE, ReadonlyRouteAuth, TWO_FACTOR_AUTH_COOKIE, TWO_FACTOR_AUTH_SECS,
    TWO_FACTOR_PENDING_COOKIE, auth_cookie, constant_time_compare_bytes, cookie_value,
    decode_totp_secret, expire_cookie, matching_totp_steps, secure_cookies,
};
use crate::qr::qr_svg_for_text;
use nodelite_proto::ReadonlyAuthConfig;

use super::{
    ChangePasswordRequest, DisableTwoFactorRequest, EnableTwoFactorRequest, SettingsActionResponse,
    TwoFactorSetupResponse, generate_totp_secret, otpauth_uri, persist_auth_2fa_change,
    persist_auth_password_change, settings_json_error, validate_password_for_settings,
};

/// 修改只读面板密码:需要当前密码,同时更新运行时鉴权与 server.toml。
pub(crate) async fn change_readonly_password(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<ChangePasswordRequest>,
) -> Response {
    super::config_edit::with_settings_write(state, move |state| {
        change_readonly_password_inner(state, headers, request)
    })
    .await
}

async fn change_readonly_password_inner(
    state: AppState,
    headers: HeaderMap,
    request: ChangePasswordRequest,
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
        return settings_json_error(StatusCode::FORBIDDEN, "current password is incorrect");
    }
    if let Some(response) = settings_confirmation_error_for_sensitive_action(
        &state,
        &current_auth,
        &headers,
        Some(&request.current_password),
        None,
    ) {
        return response;
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
        auth.revoked.cancel();
        *auth = ReadonlyRouteAuth::from_config(Some(next_auth));
        state.two_factor_sessions.clear_authenticated();
    }
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

pub(super) fn settings_confirmation_error_for_sensitive_action(
    state: &AppState,
    auth: &ReadonlyAuthConfig,
    headers: &HeaderMap,
    current_password: Option<&str>,
    code: Option<&str>,
) -> Option<Response> {
    let token = sensitive_session_token(headers, auth.enable_2fa);
    if token.as_deref().is_some_and(|token| {
        state
            .two_factor_sessions
            .sensitive_action_confirmed(token, auth.enable_2fa)
    }) {
        return None;
    }
    if !auth.enable_2fa {
        let Some(current_password) = current_password.filter(|password| !password.is_empty())
        else {
            return Some(settings_json_error(
                StatusCode::PRECONDITION_REQUIRED,
                "reauth_required",
            ));
        };
        if constant_time_compare_bytes(auth.password.as_bytes(), current_password.as_bytes()) {
            if let Some(token) = token {
                state
                    .two_factor_sessions
                    .confirm_sensitive_action(&token, false);
            }
            return None;
        }
        return Some(settings_json_error(
            StatusCode::FORBIDDEN,
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
            StatusCode::PRECONDITION_REQUIRED,
            "reauth_required",
        ));
    };
    match verify_and_consume_totp_steps(state, &secret, code) {
        Ok(()) => {}
        Err(TotpVerificationError::Invalid) => {
            return Some(settings_json_error(
                StatusCode::FORBIDDEN,
                "invalid verification code",
            ));
        }
        Err(TotpVerificationError::AlreadyUsed) => {
            return Some(settings_json_error(
                StatusCode::FORBIDDEN,
                "verification code already used",
            ));
        }
    }
    if let Some(token) = token {
        state
            .two_factor_sessions
            .confirm_sensitive_action(&token, true);
    }
    None
}

pub(super) fn sensitive_session_token(headers: &HeaderMap, two_factor: bool) -> Option<String> {
    cookie_value(
        headers,
        if two_factor {
            TWO_FACTOR_AUTH_COOKIE
        } else {
            BASIC_AUTH_SESSION_COOKIE
        },
    )
}

pub(super) enum TotpVerificationError {
    Invalid,
    AlreadyUsed,
}

pub(super) fn verify_and_consume_totp_steps(
    state: &AppState,
    secret: &[u8],
    code: &str,
) -> Result<(), TotpVerificationError> {
    let matching_steps = matching_totp_steps(Some(secret), code);
    if matching_steps.is_empty() {
        return Err(TotpVerificationError::Invalid);
    }
    if !state
        .two_factor_sessions
        .try_mark_unused_totp_steps(&matching_steps)
    {
        return Err(TotpVerificationError::AlreadyUsed);
    }
    Ok(())
}

/// 开始网页端 2FA 绑定:生成一个新 TOTP secret 和本地 SVG 二维码。
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
    super::config_edit::with_settings_write(state, move |state| {
        enable_two_factor_inner(state, request)
    })
    .await
}

async fn enable_two_factor_inner(state: AppState, request: EnableTwoFactorRequest) -> Response {
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
        return settings_json_error(StatusCode::FORBIDDEN, "current password is incorrect");
    }
    let secret = request.secret.replace(' ', "").to_ascii_uppercase();
    let Some(secret_bytes) = decode_totp_secret(&secret) else {
        return settings_json_error(StatusCode::BAD_REQUEST, "invalid TOTP secret");
    };
    if secret_bytes.len() < 10 {
        return settings_json_error(StatusCode::BAD_REQUEST, "invalid TOTP secret");
    }
    match verify_and_consume_totp_steps(&state, &secret_bytes, &request.code) {
        Ok(()) => {}
        Err(TotpVerificationError::Invalid) => {
            return settings_json_error(StatusCode::FORBIDDEN, "invalid verification code");
        }
        Err(TotpVerificationError::AlreadyUsed) => {
            return settings_json_error(StatusCode::FORBIDDEN, "verification code already used");
        }
    }

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
        auth.revoked.cancel();
        *auth = ReadonlyRouteAuth::from_config(Some(next_auth));
        state.two_factor_sessions.clear_authenticated();
    }
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

/// 关闭 2FA:要求当前密码 + 最近的 TOTP/Passkey 确认,避免无人值守浏览器直接降级。
pub(crate) async fn disable_two_factor(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<DisableTwoFactorRequest>,
) -> Response {
    super::config_edit::with_settings_write(state, move |state| {
        disable_two_factor_inner(state, headers, request)
    })
    .await
}

async fn disable_two_factor_inner(
    state: AppState,
    headers: HeaderMap,
    request: DisableTwoFactorRequest,
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
        return settings_json_error(StatusCode::FORBIDDEN, "current password is incorrect");
    }
    if let Some(response) = settings_confirmation_error_for_sensitive_action(
        &state,
        &current_auth,
        &headers,
        None,
        request.code.as_deref(),
    ) {
        return response;
    }

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
        auth.revoked.cancel();
        *auth = ReadonlyRouteAuth::from_config(Some(next_auth));
        state.two_factor_sessions.clear_authenticated();
    }
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
