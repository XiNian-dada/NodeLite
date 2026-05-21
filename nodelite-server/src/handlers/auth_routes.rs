use std::net::SocketAddr;

use axum::extract::{ConnectInfo, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::middleware::Next;
use axum::response::{AppendHeaders, IntoResponse, Response};
use axum::{Json, extract::Request};
use serde_json::json;
use tracing::error;

use crate::AppState;
use crate::admission::resolve_client_ip;
use crate::audit::{AuditEventType, NewAuditEvent};
use crate::auth::{
    TWO_FACTOR_AUTH_COOKIE, TWO_FACTOR_AUTH_SECS, TWO_FACTOR_PENDING_COOKIE,
    TWO_FACTOR_PENDING_SECS, Verify2FAError, Verify2FARequest, auth_cookie, cookie_value,
    expire_cookie, secure_cookies, verify_totp_step,
};

/// 2FA 验证 API:验证 TOTP 码,成功后设置完整认证 cookie。
pub(crate) async fn verify_2fa_api(
    State(state): State<AppState>,
    ConnectInfo(peer_addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(request): Json<Verify2FARequest>,
) -> Result<impl IntoResponse, (StatusCode, Json<Verify2FAError>)> {
    let client_ip = resolve_client_ip(state.shared.config().listen, peer_addr, &headers);
    if let Err(retry_after_secs) = state.verify_2fa_admission.check(client_ip) {
        let mut event = NewAuditEvent::now(
            AuditEventType::RateLimitExceeded,
            client_ip.to_string(),
            false,
        );
        event.user_agent = user_agent(&headers);
        event.details = json!({
            "endpoint": "/api/verify-2fa",
            "retry_after_secs": retry_after_secs,
            "reason": "verify_2fa_block",
        });
        state.audit_log.record_best_effort(event).await;
        return Err((
            StatusCode::TOO_MANY_REQUESTS,
            Json(Verify2FAError {
                error: format!("Too many failed attempts; retry after {retry_after_secs}s"),
            }),
        ));
    }

    let Some(pending_token) = cookie_value(&headers, TWO_FACTOR_PENDING_COOKIE) else {
        state.verify_2fa_admission.record_auth_failure(client_ip);
        let mut event = NewAuditEvent::now(
            AuditEventType::TotpVerifyFailure,
            client_ip.to_string(),
            false,
        );
        event.user_agent = user_agent(&headers);
        event.details = json!({
            "reason": "missing_pending_token",
        });
        state.audit_log.record_best_effort(event).await;
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(Verify2FAError {
                error: "Verification failed".to_string(),
            }),
        ));
    };

    if !state.two_factor_sessions.pending_exists(&pending_token) {
        state.verify_2fa_admission.record_auth_failure(client_ip);
        let mut event = NewAuditEvent::now(
            AuditEventType::TotpVerifyFailure,
            client_ip.to_string(),
            false,
        );
        event.user_agent = user_agent(&headers);
        event.details = json!({
            "reason": "unknown_pending_token",
        });
        state.audit_log.record_best_effort(event).await;
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(Verify2FAError {
                error: "Verification failed".to_string(),
            }),
        ));
    }

    let totp_secret = {
        let auth = state.readonly_auth.read().await;
        auth.totp_secret.clone()
    };
    let totp_step = verify_totp_step(totp_secret.as_deref(), &request.code);
    let totp_step = totp_step.filter(|step| !state.two_factor_sessions.is_totp_step_used(*step));
    let Some(totp_step) = totp_step else {
        let pending_invalidated = state
            .two_factor_sessions
            .record_failed_attempt(&pending_token);
        state.verify_2fa_admission.record_auth_failure(client_ip);
        let mut event = NewAuditEvent::now(
            AuditEventType::TotpVerifyFailure,
            client_ip.to_string(),
            false,
        );
        event.user_agent = user_agent(&headers);
        event.details = json!({
            "reason": "invalid_or_replayed_totp",
            "pending_invalidated": pending_invalidated,
        });
        state.audit_log.record_best_effort(event).await;
        let secure = secure_cookies(state.shared.config());
        let body = Json(Verify2FAError {
            error: "Verification failed".to_string(),
        });
        let response = if pending_invalidated {
            (
                StatusCode::UNAUTHORIZED,
                AppendHeaders([expire_cookie(TWO_FACTOR_PENDING_COOKIE, secure)]),
                body,
            )
                .into_response()
        } else {
            (StatusCode::UNAUTHORIZED, body).into_response()
        };
        return Ok(response);
    };
    state.two_factor_sessions.mark_totp_step_used(totp_step);

    let audit_user = {
        let auth = state.readonly_auth.read().await;
        auth.config.as_ref().map(|config| config.username.clone())
    };
    let auth_token = state
        .two_factor_sessions
        .create_authenticated()
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(Verify2FAError {
                    error: "Failed to create authenticated session".to_string(),
                }),
            )
        })?;
    state.two_factor_sessions.consume_pending(&pending_token);
    state.verify_2fa_admission.clear_auth_failures(client_ip);
    let mut event = NewAuditEvent::now(
        AuditEventType::TotpVerifySuccess,
        client_ip.to_string(),
        true,
    );
    event.user = audit_user;
    event.user_agent = user_agent(&headers);
    event.details = json!({
        "endpoint": "/api/verify-2fa",
    });
    state.audit_log.record_best_effort(event).await;
    let secure = secure_cookies(state.shared.config());

    Ok((
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
    )
        .into_response())
}

/// 健康检查接口,始终返回 200。
pub(crate) async fn healthz() -> StatusCode {
    StatusCode::OK
}

/// 就绪检查接口:仅当关键依赖均可用时返回 200,否则返回 503。
pub(crate) async fn readyz(State(state): State<AppState>) -> StatusCode {
    if state.readiness.is_ready() {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    }
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

/// 中间件:对受保护路由强制基本认证;放行时把 Request 继续交给下一个处理器。
pub(crate) async fn require_readonly_auth(
    State(state): State<AppState>,
    headers: HeaderMap,
    request: Request,
    next: Next,
) -> Response {
    let audit_ip =
        request_client_ip(&state, &headers, &request).unwrap_or_else(|| "unknown".to_string());
    let audit_user_agent = user_agent(&headers);
    let auth = state.readonly_auth.read().await;

    if auth.expected_authorization.is_none() {
        drop(auth);
        return next.run(request).await;
    }

    if auth.enable_2fa {
        if cookie_value(&headers, TWO_FACTOR_AUTH_COOKIE)
            .as_deref()
            .is_some_and(|token| state.two_factor_sessions.is_authenticated(token))
        {
            drop(auth);
            return next.run(request).await;
        }

        if auth.is_authorized(&request) {
            drop(auth);
            let pending_token = match state.two_factor_sessions.create_pending() {
                Ok(token) => token,
                Err(error) => {
                    error!(error = ?error, "failed to create pending 2FA session");
                    return StatusCode::INTERNAL_SERVER_ERROR.into_response();
                }
            };
            let secure = secure_cookies(state.shared.config());

            return (
                StatusCode::FOUND,
                AppendHeaders([
                    auth_cookie(
                        TWO_FACTOR_PENDING_COOKIE,
                        &pending_token,
                        TWO_FACTOR_PENDING_SECS,
                        secure,
                    ),
                    expire_cookie(TWO_FACTOR_AUTH_COOKIE, secure),
                    (header::LOCATION, "/verify-2fa".to_string()),
                ]),
            )
                .into_response();
        }
    } else if auth.is_authorized(&request) {
        drop(auth);
        return next.run(request).await;
    }

    let two_factor_enabled = auth.enable_2fa;
    drop(auth);
    let mut event = NewAuditEvent::now(AuditEventType::LoginFailure, audit_ip, false);
    event.user_agent = audit_user_agent;
    event.details = json!({
        "path": request.uri().path(),
        "reason": if headers.contains_key(header::AUTHORIZATION) {
            "invalid_basic_auth"
        } else {
            "missing_basic_auth"
        },
        "two_factor_enabled": two_factor_enabled,
    });
    state.audit_log.record_best_effort(event).await;

    (
        StatusCode::UNAUTHORIZED,
        [(header::WWW_AUTHENTICATE, "Basic realm=\"NodeLite\"")],
        "authentication required",
    )
        .into_response()
}

fn user_agent(headers: &HeaderMap) -> Option<String> {
    headers
        .get(header::USER_AGENT)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string)
}

fn request_client_ip(state: &AppState, headers: &HeaderMap, request: &Request) -> Option<String> {
    request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|connect_info| {
            resolve_client_ip(state.shared.config().listen, connect_info.0, headers).to_string()
        })
}
