//! Short-lived, session-bound confirmation for sensitive settings changes.

use std::net::{IpAddr, SocketAddr};

use axum::Json;
use axum::extract::{ConnectInfo, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use webauthn_rs::prelude::PublicKeyCredential;

use crate::AppState;
use crate::admission::resolve_client_ip;
use crate::auth::{constant_time_compare_bytes, decode_totp_secret};
use crate::passkeys::PasskeyError;

use super::security::{sensitive_session_token, verify_and_consume_totp_steps};
use super::{SettingsActionResponse, settings_json_error};

#[derive(Deserialize)]
pub(crate) struct ConfirmSettingsRequest {
    #[serde(default)]
    current_password: Option<String>,
    #[serde(default)]
    code: Option<String>,
}

pub(crate) async fn confirm_settings(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(request): Json<ConfirmSettingsRequest>,
) -> Response {
    let ip = client_ip(&state, peer, &headers);
    if let Some(response) = blocked(&state, ip) {
        return response;
    }
    let auth = state.readonly_auth.read().await;
    let Some(config) = auth.config.as_ref() else {
        return settings_json_error(StatusCode::CONFLICT, "readonly auth is not enabled");
    };
    let Some(token) = sensitive_session_token(&headers, config.enable_2fa) else {
        return settings_json_error(
            StatusCode::UNAUTHORIZED,
            "authenticated session is required",
        );
    };
    let valid = if config.enable_2fa {
        config
            .totp_secret
            .as_deref()
            .and_then(decode_totp_secret)
            .zip(request.code.as_deref())
            .is_some_and(|(secret, code)| {
                verify_and_consume_totp_steps(&state, &secret, code.trim()).is_ok()
            })
    } else {
        request.current_password.as_deref().is_some_and(|password| {
            constant_time_compare_bytes(config.password.as_bytes(), password.as_bytes())
        })
    };
    let two_factor = config.enable_2fa;
    drop(auth);
    if !valid {
        state
            .sensitive_readonly_auth_admission
            .record_auth_failure(ip);
        return settings_json_error(StatusCode::FORBIDDEN, "confirmation was not accepted");
    }
    confirmed_response(&state, &token, two_factor, ip)
}

pub(crate) async fn start_settings_passkey_confirmation(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
) -> Response {
    let ip = client_ip(&state, peer, &headers);
    if let Some(response) = blocked(&state, ip) {
        return response;
    }
    let auth = state.readonly_auth.read().await;
    if !auth.enable_2fa {
        return settings_json_error(StatusCode::CONFLICT, "passkey confirmation requires 2FA");
    }
    let Some(token) = sensitive_session_token(&headers, true) else {
        return settings_json_error(
            StatusCode::UNAUTHORIZED,
            "authenticated session is required",
        );
    };
    drop(auth);
    if !state.two_factor_sessions.is_authenticated(&token) {
        return settings_json_error(
            StatusCode::UNAUTHORIZED,
            "authenticated session is required",
        );
    }
    match state.passkeys.start_authentication(&token) {
        Ok(challenge) => Json(challenge).into_response(),
        Err(PasskeyError::NoCredentials | PasskeyError::InsecureOrigin) => {
            settings_json_error(StatusCode::CONFLICT, "passkeys are not available")
        }
        Err(error) => {
            tracing::error!(error = ?error, "failed to start settings passkey confirmation");
            settings_json_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "passkey confirmation failed",
            )
        }
    }
}

pub(crate) async fn finish_settings_passkey_confirmation(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(response): Json<PublicKeyCredential>,
) -> Response {
    let ip = client_ip(&state, peer, &headers);
    if let Some(response) = blocked(&state, ip) {
        return response;
    }
    let auth = state.readonly_auth.read().await;
    if !auth.enable_2fa {
        return settings_json_error(StatusCode::CONFLICT, "passkey confirmation requires 2FA");
    }
    let Some(token) = sensitive_session_token(&headers, true) else {
        return settings_json_error(
            StatusCode::UNAUTHORIZED,
            "authenticated session is required",
        );
    };
    drop(auth);
    if !state.two_factor_sessions.is_authenticated(&token) {
        return settings_json_error(
            StatusCode::UNAUTHORIZED,
            "authenticated session is required",
        );
    }
    if let Err(error) = state
        .passkeys
        .finish_authentication(&token, &response)
        .await
    {
        state
            .sensitive_readonly_auth_admission
            .record_auth_failure(ip);
        tracing::warn!(error = ?error, "settings passkey confirmation rejected");
        return settings_json_error(
            StatusCode::FORBIDDEN,
            "passkey confirmation was not accepted",
        );
    }
    confirmed_response(&state, &token, true, ip)
}

fn confirmed_response(state: &AppState, token: &str, two_factor: bool, ip: IpAddr) -> Response {
    if !state
        .two_factor_sessions
        .confirm_sensitive_action(token, two_factor)
    {
        return settings_json_error(StatusCode::UNAUTHORIZED, "authenticated session expired");
    }
    state
        .sensitive_readonly_auth_admission
        .clear_auth_failures(ip);
    Json(SettingsActionResponse {
        ok: true,
        message: "confirmed for five minutes".to_string(),
    })
    .into_response()
}

fn client_ip(state: &AppState, peer: SocketAddr, headers: &HeaderMap) -> IpAddr {
    resolve_client_ip(&state.shared.config().trusted_proxies, peer, headers)
}

fn blocked(state: &AppState, ip: IpAddr) -> Option<Response> {
    state
        .sensitive_readonly_auth_admission
        .check(ip)
        .err()
        .map(|secs| {
            settings_json_error(
                StatusCode::TOO_MANY_REQUESTS,
                format!("try again in {secs} seconds"),
            )
        })
}
