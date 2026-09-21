//! Authenticated account settings endpoints for enrolling and revoking passkeys.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use tracing::error;
use uuid::Uuid;
use webauthn_rs::prelude::RegisterPublicKeyCredential;

use crate::AppState;
use crate::auth::{TWO_FACTOR_AUTH_COOKIE, cookie_value};
use crate::passkeys::PasskeyError;

use super::security::settings_confirmation_error_for_sensitive_action;
use super::{
    DeletePasskeyRequest, SettingsActionResponse, StartPasskeyRegistrationRequest,
    settings_json_error,
};

/// Starts an enrollment ceremony after a fresh TOTP confirmation.
pub(crate) async fn start_passkey_registration(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<StartPasskeyRegistrationRequest>,
) -> Response {
    let (username, session_binding) =
        match confirmed_passkey_session(&state, &headers, &request).await {
            Ok(values) => values,
            Err(response) => return *response,
        };
    match state
        .passkeys
        .start_registration(&username, &session_binding, &request.label)
    {
        Ok(challenge) => Json(challenge).into_response(),
        Err(error) => passkey_operation_error("start registration", error),
    }
}

/// Completes an enrollment ceremony that was bound to the current 2FA session.
pub(crate) async fn finish_passkey_registration(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(response): Json<RegisterPublicKeyCredential>,
) -> Response {
    let Some(session_binding) = authenticated_session_binding(&state, &headers) else {
        return settings_json_error(
            StatusCode::UNAUTHORIZED,
            "authenticated session is required",
        );
    };
    match state
        .passkeys
        .finish_registration(&session_binding, &response)
        .await
    {
        Ok(summary) => (StatusCode::CREATED, Json(summary)).into_response(),
        Err(error) => passkey_operation_error("finish registration", error),
    }
}

/// Removes one enrolled passkey after a fresh TOTP confirmation.
pub(crate) async fn delete_passkey(
    State(state): State<AppState>,
    Path(passkey_id): Path<Uuid>,
    headers: HeaderMap,
    Json(request): Json<DeletePasskeyRequest>,
) -> Response {
    let current_auth = {
        let auth = state.readonly_auth.read().await;
        auth.config.clone()
    };
    let Some(current_auth) = current_auth else {
        return settings_json_error(StatusCode::CONFLICT, "readonly auth is not enabled");
    };
    if !current_auth.enable_2fa {
        return settings_json_error(
            StatusCode::CONFLICT,
            "enable two-factor authentication before managing passkeys",
        );
    }
    if authenticated_session_binding(&state, &headers).is_none() {
        return settings_json_error(
            StatusCode::UNAUTHORIZED,
            "authenticated session is required",
        );
    }
    if let Some(response) = settings_confirmation_error_for_sensitive_action(
        &state,
        &current_auth,
        request.current_password.as_deref(),
        request.code.as_deref(),
    ) {
        return response;
    }
    match state.passkeys.delete(passkey_id).await {
        Ok(()) => Json(SettingsActionResponse {
            ok: true,
            message: "passkey removed".to_string(),
        })
        .into_response(),
        Err(PasskeyError::CredentialNotFound) => {
            settings_json_error(StatusCode::NOT_FOUND, "passkey not found")
        }
        Err(error) => passkey_operation_error("remove passkey", error),
    }
}

async fn confirmed_passkey_session(
    state: &AppState,
    headers: &HeaderMap,
    request: &StartPasskeyRegistrationRequest,
) -> Result<(String, String), Box<Response>> {
    let current_auth = {
        let auth = state.readonly_auth.read().await;
        auth.config.clone()
    };
    let Some(current_auth) = current_auth else {
        return Err(Box::new(settings_json_error(
            StatusCode::CONFLICT,
            "readonly auth is not enabled",
        )));
    };
    if !current_auth.enable_2fa {
        return Err(Box::new(settings_json_error(
            StatusCode::CONFLICT,
            "enable two-factor authentication before adding a passkey",
        )));
    }
    let Some(session_binding) = authenticated_session_binding(state, headers) else {
        return Err(Box::new(settings_json_error(
            StatusCode::UNAUTHORIZED,
            "authenticated session is required",
        )));
    };
    if let Some(response) = settings_confirmation_error_for_sensitive_action(
        state,
        &current_auth,
        request.current_password.as_deref(),
        request.code.as_deref(),
    ) {
        return Err(Box::new(response));
    }
    Ok((current_auth.username, session_binding))
}

fn authenticated_session_binding(state: &AppState, headers: &HeaderMap) -> Option<String> {
    let token = cookie_value(headers, TWO_FACTOR_AUTH_COOKIE)?;
    state
        .two_factor_sessions
        .is_authenticated(&token)
        .then_some(token)
}

fn passkey_operation_error(operation: &str, error: PasskeyError) -> Response {
    match error {
        PasskeyError::InsecureOrigin => settings_json_error(
            StatusCode::CONFLICT,
            "passkeys require server.public_base_url to use https://",
        ),
        PasskeyError::InvalidLabel => {
            settings_json_error(StatusCode::BAD_REQUEST, "invalid passkey label")
        }
        PasskeyError::CredentialLimit => settings_json_error(
            StatusCode::CONFLICT,
            "the maximum number of passkeys has been reached",
        ),
        PasskeyError::RegistrationExpired => settings_json_error(
            StatusCode::UNAUTHORIZED,
            "passkey registration has expired; try again",
        ),
        PasskeyError::Operation | PasskeyError::DuplicateCredential => settings_json_error(
            StatusCode::BAD_REQUEST,
            "passkey registration was not accepted",
        ),
        error => {
            error!(error = ?error, operation, "passkey settings operation failed");
            settings_json_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "passkey operation failed",
            )
        }
    }
}
