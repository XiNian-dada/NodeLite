//! 2FA 验证 API 与认证 cookie 路由。

use std::net::SocketAddr;

use axum::Json;
use axum::extract::{ConnectInfo, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{AppendHeaders, IntoResponse, Response};
use serde_json::json;

use super::user_agent;
use crate::AppState;
use crate::admission::resolve_client_ip;
use crate::audit::{AuditEventType, NewAuditEvent};
use crate::auth::{
    TWO_FACTOR_AUTH_COOKIE, TWO_FACTOR_AUTH_SECS, TWO_FACTOR_PENDING_COOKIE, Verify2FAError,
    Verify2FARequest, auth_cookie, cookie_value, expire_cookie, matching_totp_steps,
    secure_cookies,
};
use crate::handlers::record_audit_event;

type Verify2FAResult<T> = Result<T, (StatusCode, Json<Verify2FAError>)>;

/// 2FA 验证 API:验证 TOTP 码,成功后设置完整认证 cookie。
pub(crate) async fn verify_2fa_api(
    State(state): State<AppState>,
    ConnectInfo(peer_addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(request): Json<Verify2FARequest>,
) -> Verify2FAResult<Response> {
    let client_ip = resolve_client_ip(&state.shared.config().trusted_proxies, peer_addr, &headers);
    ensure_verify_2fa_not_blocked(&state, &headers, client_ip).await?;
    let pending_token = require_pending_token(&state, &headers, client_ip).await?;
    let Some(totp_steps) = current_unused_totp_steps(&state, &request.code).await else {
        return Ok(handle_invalid_totp(&state, &headers, client_ip, &pending_token).await);
    };
    mark_totp_steps_used(&state, &totp_steps);

    let audit_user = readonly_auth_username(&state).await;
    let auth_token = create_authenticated_two_factor_session(&state)?;
    state.two_factor_sessions.consume_pending(&pending_token);
    state.verify_2fa_admission.clear_auth_failures(client_ip);
    record_totp_success(&state, &headers, client_ip, audit_user).await;

    Ok(successful_verify_2fa_response(&state, &auth_token))
}

/// 登出并强制重新认证:返回 401 + WWW-Authenticate 头,触发浏览器清除缓存的
/// Basic Auth 凭据。前端在检测到认证过期(24 小时)时会跳转到此路由。
pub(crate) async fn logout_and_reauth(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Response {
    if let Some(token) = cookie_value(&headers, TWO_FACTOR_AUTH_COOKIE) {
        state.two_factor_sessions.remove_authenticated(&token);
    }
    let secure = secure_cookies(state.shared.config());
    (
        StatusCode::UNAUTHORIZED,
        AppendHeaders([
            expire_cookie(TWO_FACTOR_AUTH_COOKIE, secure),
            expire_cookie(TWO_FACTOR_PENDING_COOKIE, secure),
            (
                header::WWW_AUTHENTICATE,
                "Basic realm=\"NodeLite\"".to_string(),
            ),
        ]),
        "Session expired. Please log in again.",
    )
        .into_response()
}

async fn ensure_verify_2fa_not_blocked(
    state: &AppState,
    headers: &HeaderMap,
    client_ip: std::net::IpAddr,
) -> Verify2FAResult<()> {
    let Err(retry_after_secs) = state.verify_2fa_admission.check(client_ip) else {
        return Ok(());
    };
    record_audit_event(
        state,
        AuditEventType::RateLimitExceeded,
        client_ip.to_string(),
        false,
        user_agent(headers),
        json!({
            "endpoint": "/api/verify-2fa",
            "retry_after_secs": retry_after_secs,
            "reason": "verify_2fa_block",
        }),
    )
    .await;
    Err((
        StatusCode::TOO_MANY_REQUESTS,
        Json(Verify2FAError {
            error: format!("Too many failed attempts; retry after {retry_after_secs}s"),
        }),
    ))
}

async fn require_pending_token(
    state: &AppState,
    headers: &HeaderMap,
    client_ip: std::net::IpAddr,
) -> Verify2FAResult<String> {
    let Some(pending_token) = cookie_value(headers, TWO_FACTOR_PENDING_COOKIE) else {
        record_totp_failure(
            state,
            headers,
            client_ip,
            json!({ "reason": "missing_pending_token" }),
        )
        .await;
        return Err(verify_2fa_unauthorized_error());
    };
    if !state.two_factor_sessions.pending_exists(&pending_token) {
        record_totp_failure(
            state,
            headers,
            client_ip,
            json!({ "reason": "unknown_pending_token" }),
        )
        .await;
        return Err(verify_2fa_unauthorized_error());
    }
    Ok(pending_token)
}

async fn current_unused_totp_steps(state: &AppState, code: &str) -> Option<Vec<u64>> {
    let totp_secret = {
        let auth = state.readonly_auth.read().await;
        auth.totp_secret.clone()
    };
    let matching_steps = matching_totp_steps(totp_secret.as_deref(), code);
    if matching_steps.is_empty()
        || matching_steps
            .iter()
            .all(|step| state.two_factor_sessions.is_totp_step_used(*step))
    {
        return None;
    }
    Some(matching_steps)
}

fn mark_totp_steps_used(state: &AppState, steps: &[u64]) {
    for step in steps {
        state.two_factor_sessions.mark_totp_step_used(*step);
    }
}

async fn handle_invalid_totp(
    state: &AppState,
    headers: &HeaderMap,
    client_ip: std::net::IpAddr,
    pending_token: &str,
) -> Response {
    let pending_invalidated = state
        .two_factor_sessions
        .record_failed_attempt(pending_token);
    record_totp_failure(
        state,
        headers,
        client_ip,
        json!({
            "reason": "invalid_or_replayed_totp",
            "pending_invalidated": pending_invalidated,
        }),
    )
    .await;
    let secure = secure_cookies(state.shared.config());
    let body = Json(Verify2FAError {
        error: "Verification failed".to_string(),
    });
    if pending_invalidated {
        (
            StatusCode::UNAUTHORIZED,
            AppendHeaders([expire_cookie(TWO_FACTOR_PENDING_COOKIE, secure)]),
            body,
        )
            .into_response()
    } else {
        (StatusCode::UNAUTHORIZED, body).into_response()
    }
}

async fn record_totp_failure(
    state: &AppState,
    headers: &HeaderMap,
    client_ip: std::net::IpAddr,
    details: serde_json::Value,
) {
    state.verify_2fa_admission.record_auth_failure(client_ip);
    record_audit_event(
        state,
        AuditEventType::TotpVerifyFailure,
        client_ip.to_string(),
        false,
        user_agent(headers),
        details,
    )
    .await;
}

async fn readonly_auth_username(state: &AppState) -> Option<String> {
    let auth = state.readonly_auth.read().await;
    auth.config.as_ref().map(|config| config.username.clone())
}

fn create_authenticated_two_factor_session(state: &AppState) -> Verify2FAResult<String> {
    state
        .two_factor_sessions
        .create_authenticated()
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(Verify2FAError {
                    error: "Failed to create authenticated session".to_string(),
                }),
            )
        })
}

async fn record_totp_success(
    state: &AppState,
    headers: &HeaderMap,
    client_ip: std::net::IpAddr,
    audit_user: Option<String>,
) {
    let mut event = NewAuditEvent::now(
        AuditEventType::TotpVerifySuccess,
        client_ip.to_string(),
        true,
    );
    event.user = audit_user;
    event.user_agent = user_agent(headers);
    event.details = json!({
        "endpoint": "/api/verify-2fa",
    });
    state.audit_log.record_best_effort(event).await;
}

fn successful_verify_2fa_response(state: &AppState, auth_token: &str) -> Response {
    let secure = secure_cookies(state.shared.config());
    (
        StatusCode::OK,
        AppendHeaders([
            auth_cookie(
                TWO_FACTOR_AUTH_COOKIE,
                auth_token,
                TWO_FACTOR_AUTH_SECS,
                secure,
            ),
            expire_cookie(TWO_FACTOR_PENDING_COOKIE, secure),
        ]),
    )
        .into_response()
}

fn verify_2fa_unauthorized_error() -> (StatusCode, Json<Verify2FAError>) {
    (
        StatusCode::UNAUTHORIZED,
        Json(Verify2FAError {
            error: "Verification failed".to_string(),
        }),
    )
}
