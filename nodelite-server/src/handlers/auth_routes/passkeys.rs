//! Public passkey authentication endpoints for a pending browser login.

use std::net::SocketAddr;

use axum::Json;
use axum::extract::{ConnectInfo, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use serde_json::json;
use tracing::error;
use webauthn_rs::prelude::{PublicKeyCredential, RequestChallengeResponse};

use super::two_factor::{
    Verify2FAResult, ensure_second_factor_not_blocked, record_second_factor_failure,
    record_second_factor_success, second_factor_unauthorized_error,
    successful_second_factor_response,
};
use crate::AppState;
use crate::admission::resolve_client_ip;
use crate::audit::AuditEventType;
use crate::auth::{TWO_FACTOR_PENDING_COOKIE, Verify2FAError, cookie_value};
use crate::passkeys::PasskeyError;

/// Starts a user-verifying WebAuthn authentication ceremony for the pending login.
pub(crate) async fn start_passkey_authentication(
    State(state): State<AppState>,
    ConnectInfo(peer_addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
) -> Verify2FAResult<Json<RequestChallengeResponse>> {
    let client_ip = resolve_client_ip(&state.shared.config().trusted_proxies, peer_addr, &headers);
    let endpoint = "/api/passkeys/authentication/start";
    ensure_second_factor_not_blocked(&state, &headers, client_ip, endpoint).await?;
    let pending_token = require_pending_token(&state, &headers, client_ip, endpoint).await?;

    if !passkey_login_is_enabled(&state).await {
        return Err(second_factor_unauthorized_error());
    }

    match state.passkeys.start_authentication(&pending_token) {
        Ok(challenge) => Ok(Json(challenge)),
        Err(PasskeyError::NoCredentials | PasskeyError::InsecureOrigin) => Err((
            StatusCode::CONFLICT,
            Json(Verify2FAError {
                error: "Passkeys are not available".to_string(),
            }),
        )),
        Err(error) => {
            error!(error = ?error, "failed to start passkey authentication");
            Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(Verify2FAError {
                    error: "Failed to start passkey authentication".to_string(),
                }),
            ))
        }
    }
}

/// Verifies the WebAuthn assertion and exchanges the pending login for an auth session.
pub(crate) async fn finish_passkey_authentication(
    State(state): State<AppState>,
    ConnectInfo(peer_addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(response): Json<PublicKeyCredential>,
) -> Verify2FAResult<Response> {
    let client_ip = resolve_client_ip(&state.shared.config().trusted_proxies, peer_addr, &headers);
    let endpoint = "/api/passkeys/authentication/finish";
    ensure_second_factor_not_blocked(&state, &headers, client_ip, endpoint).await?;
    let pending_token = require_pending_token(&state, &headers, client_ip, endpoint).await?;
    let Some(audit_user) = passkey_login_user(&state).await else {
        return Err(second_factor_unauthorized_error());
    };

    if let Err(error) = state
        .passkeys
        .finish_authentication(&pending_token, &response)
        .await
    {
        return handle_passkey_failure(&state, &headers, client_ip, endpoint, error).await;
    }

    let auth_token = state
        .two_factor_sessions
        .exchange_pending_for_passkey(&pending_token)
        .map_err(|error| {
            error!(error = ?error, "failed to create passkey-authenticated session");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(Verify2FAError {
                    error: "Failed to create authenticated session".to_string(),
                }),
            )
        })?;
    let Some(auth_token) = auth_token else {
        record_passkey_failure(
            &state,
            &headers,
            client_ip,
            json!({ "endpoint": endpoint, "reason": "pending_login_consumed" }),
        )
        .await;
        return Err(second_factor_unauthorized_error());
    };

    state.verify_2fa_admission.clear_auth_failures(client_ip);
    let login_event_id = record_second_factor_success(
        &state,
        &headers,
        client_ip,
        Some(audit_user),
        AuditEventType::PasskeyVerifySuccess,
        endpoint,
        "passkey",
    )
    .await;
    state
        .two_factor_sessions
        .set_authenticated_login_event_id(&auth_token, login_event_id);
    Ok(successful_second_factor_response(&state, &auth_token))
}

async fn require_pending_token(
    state: &AppState,
    headers: &HeaderMap,
    client_ip: std::net::IpAddr,
    endpoint: &str,
) -> Verify2FAResult<String> {
    let Some(pending_token) = cookie_value(headers, TWO_FACTOR_PENDING_COOKIE) else {
        record_passkey_failure(
            state,
            headers,
            client_ip,
            json!({ "endpoint": endpoint, "reason": "missing_pending_token" }),
        )
        .await;
        return Err(second_factor_unauthorized_error());
    };
    if !state.two_factor_sessions.pending_exists(&pending_token) {
        record_passkey_failure(
            state,
            headers,
            client_ip,
            json!({ "endpoint": endpoint, "reason": "unknown_pending_token" }),
        )
        .await;
        return Err(second_factor_unauthorized_error());
    }
    Ok(pending_token)
}

async fn passkey_login_is_enabled(state: &AppState) -> bool {
    let auth = state.readonly_auth.read().await;
    auth.enable_2fa && auth.config.is_some()
}

async fn passkey_login_user(state: &AppState) -> Option<String> {
    let auth = state.readonly_auth.read().await;
    auth.enable_2fa
        .then(|| auth.config.as_ref().map(|config| config.username.clone()))
        .flatten()
}

async fn handle_passkey_failure(
    state: &AppState,
    headers: &HeaderMap,
    client_ip: std::net::IpAddr,
    endpoint: &str,
    error: PasskeyError,
) -> Verify2FAResult<Response> {
    match error {
        PasskeyError::AuthenticationExpired
        | PasskeyError::CredentialNotFound
        | PasskeyError::Operation => {
            record_passkey_failure(
                state,
                headers,
                client_ip,
                json!({ "endpoint": endpoint, "reason": "verification_failed" }),
            )
            .await;
            Err(second_factor_unauthorized_error())
        }
        error => {
            error!(error = ?error, "passkey authentication failed unexpectedly");
            Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(Verify2FAError {
                    error: "Passkey authentication failed".to_string(),
                }),
            ))
        }
    }
}

async fn record_passkey_failure(
    state: &AppState,
    headers: &HeaderMap,
    client_ip: std::net::IpAddr,
    details: serde_json::Value,
) {
    record_second_factor_failure(
        state,
        headers,
        client_ip,
        AuditEventType::PasskeyVerifyFailure,
        details,
    )
    .await;
}
