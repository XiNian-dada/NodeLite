//! 只读路由 Basic Auth 与 2FA 中间件。

use std::net::SocketAddr;

use axum::Json;
use axum::extract::{ConnectInfo, Request, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::middleware::Next;
use axum::response::{AppendHeaders, IntoResponse, Response};
use serde_json::json;
use tracing::error;

use super::user_agent;
use crate::AppState;
use crate::admission::resolve_client_ip;
use crate::audit::AuditEventType;
use crate::auth::{
    TWO_FACTOR_AUTH_COOKIE, TWO_FACTOR_PENDING_COOKIE, TWO_FACTOR_PENDING_SECS, auth_cookie,
    cookie_value, expire_cookie, secure_cookies,
};
use crate::handlers::record_audit_event;

struct ReadonlyAuthMeta {
    request_path: String,
    sensitive_path: bool,
    client_ip: std::net::IpAddr,
    audit_ip: String,
    audit_user_agent: Option<String>,
}

/// 中间件:对受保护路由强制基本认证;放行时把 Request 继续交给下一个处理器。
pub(crate) async fn require_readonly_auth(
    State(state): State<AppState>,
    headers: HeaderMap,
    request: Request,
    next: Next,
) -> Response {
    let meta = readonly_auth_meta(&state, &headers, &request);
    let auth = state.readonly_auth.read().await;
    let evaluated = evaluate_readonly_auth(&state, &auth, &headers, &request);
    drop(auth);
    let Some((basic_authorized, two_factor_enabled)) = evaluated else {
        return next.run(request).await;
    };
    if let Some(response) = enforce_readonly_auth_limits(&state, &meta).await {
        return response;
    }
    if basic_authorized {
        clear_readonly_auth_failures(&state, &meta);
        if two_factor_enabled {
            // WebSocket 升级握手无法跟随 302 重定向(浏览器 `new WebSocket()` 不暴露
            // redirect),返回 401 + JSON。WS 客户端据此 fallback 到一次 `/api/bootstrap`
            // 探测,由统一 api 客户端完成到 /verify-2fa 的真实跳转。
            if is_websocket_upgrade(&headers) {
                return websocket_two_factor_required_response();
            }
            return issue_two_factor_redirect(&state).await;
        }
        return next.run(request).await;
    }

    record_readonly_login_failure(
        &state,
        &meta,
        headers.contains_key(header::AUTHORIZATION),
        two_factor_enabled,
    )
    .await;
    readonly_auth_unauthorized_response()
}

fn readonly_auth_meta(
    state: &AppState,
    headers: &HeaderMap,
    request: &Request,
) -> ReadonlyAuthMeta {
    let request_path = request.uri().path().to_string();
    let sensitive_path = is_sensitive_readonly_path(&request_path, request);
    let client_ip = request_client_ip(state, headers, request);
    ReadonlyAuthMeta {
        request_path,
        sensitive_path,
        audit_ip: client_ip.to_string(),
        audit_user_agent: user_agent(headers),
        client_ip,
    }
}

fn evaluate_readonly_auth(
    state: &AppState,
    auth: &crate::auth::ReadonlyRouteAuth,
    headers: &HeaderMap,
    request: &Request,
) -> Option<(bool, bool)> {
    auth.expected_authorization.as_ref()?;
    if auth.enable_2fa && has_authenticated_two_factor_cookie(state, headers) {
        return None;
    }
    Some((auth.is_authorized(request), auth.enable_2fa))
}

fn has_authenticated_two_factor_cookie(state: &AppState, headers: &HeaderMap) -> bool {
    cookie_value(headers, TWO_FACTOR_AUTH_COOKIE)
        .as_deref()
        .is_some_and(|token| state.two_factor_sessions.is_authenticated(token))
}

async fn enforce_readonly_auth_limits(
    state: &AppState,
    meta: &ReadonlyAuthMeta,
) -> Option<Response> {
    if let Err(retry_after_secs) = state.readonly_auth_admission.check(meta.client_ip) {
        record_readonly_auth_block(
            state,
            &meta.audit_ip,
            &meta.audit_user_agent,
            &meta.request_path,
            meta.sensitive_path,
            retry_after_secs,
        )
        .await;
        return Some(readonly_auth_block_response(retry_after_secs));
    }
    if meta.sensitive_path
        && let Err(retry_after_secs) = state
            .sensitive_readonly_auth_admission
            .check(meta.client_ip)
    {
        record_readonly_auth_block(
            state,
            &meta.audit_ip,
            &meta.audit_user_agent,
            &meta.request_path,
            true,
            retry_after_secs,
        )
        .await;
        return Some(readonly_auth_block_response(retry_after_secs));
    }
    None
}

fn clear_readonly_auth_failures(state: &AppState, meta: &ReadonlyAuthMeta) {
    state
        .readonly_auth_admission
        .clear_auth_failures(meta.client_ip);
    if meta.sensitive_path {
        state
            .sensitive_readonly_auth_admission
            .clear_auth_failures(meta.client_ip);
    }
}

/// 判断请求是否为 WebSocket 升级握手。依据 `Upgrade` 头是否包含 `websocket`
/// token(大小写不敏感、容忍逗号分隔),与 axum `WebSocketUpgrade` 的识别一致。
fn is_websocket_upgrade(headers: &HeaderMap) -> bool {
    headers
        .get(header::UPGRADE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| {
            value
                .split(',')
                .any(|token| token.trim().eq_ignore_ascii_case("websocket"))
        })
}

/// 受保护的 WebSocket 端点要求 2FA 时的响应。浏览器 `new WebSocket()` 无法跟随
/// HTTP 302,因此返回 401 + JSON 而非重定向;WS 客户端据此跳转 /verify-2fa。
fn websocket_two_factor_required_response() -> Response {
    (
        StatusCode::UNAUTHORIZED,
        Json(json!({
            "ok": false,
            "message": "two_factor_required",
            "endpoint": "/verify-2fa",
        })),
    )
        .into_response()
}

async fn issue_two_factor_redirect(state: &AppState) -> Response {
    let pending_token = match state.two_factor_sessions.create_pending() {
        Ok(token) => token,
        Err(error) => {
            error!(error = ?error, "failed to create pending 2FA session");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    let secure = secure_cookies(state.shared.config());
    (
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
        .into_response()
}

async fn record_readonly_login_failure(
    state: &AppState,
    meta: &ReadonlyAuthMeta,
    has_authorization_header: bool,
    two_factor_enabled: bool,
) {
    state
        .readonly_auth_admission
        .record_auth_failure(meta.client_ip);
    if meta.sensitive_path {
        state
            .sensitive_readonly_auth_admission
            .record_auth_failure(meta.client_ip);
    }
    record_audit_event(
        state,
        AuditEventType::LoginFailure,
        meta.audit_ip.clone(),
        false,
        meta.audit_user_agent.clone(),
        json!({
            "path": meta.request_path,
            "reason": if has_authorization_header {
                "invalid_basic_auth"
            } else {
                "missing_basic_auth"
            },
            "two_factor_enabled": two_factor_enabled,
            "sensitive_path": meta.sensitive_path,
        }),
    )
    .await;
}

fn readonly_auth_unauthorized_response() -> Response {
    (
        StatusCode::UNAUTHORIZED,
        [(header::WWW_AUTHENTICATE, "Basic realm=\"NodeLite\"")],
        "authentication required",
    )
        .into_response()
}

fn is_sensitive_readonly_path(path: &str, request: &Request) -> bool {
    request.method() != axum::http::Method::GET && path.starts_with("/api/settings/")
}

async fn record_readonly_auth_block(
    state: &AppState,
    audit_ip: &str,
    audit_user_agent: &Option<String>,
    request_path: &str,
    sensitive_path: bool,
    retry_after_secs: u64,
) {
    record_audit_event(
        state,
        AuditEventType::RateLimitExceeded,
        audit_ip.to_string(),
        false,
        audit_user_agent.clone(),
        json!({
            "path": request_path,
            "reason": "readonly_auth_block",
            "retry_after_secs": retry_after_secs,
            "sensitive_path": sensitive_path,
        }),
    )
    .await;
}

fn readonly_auth_block_response(retry_after_secs: u64) -> Response {
    (
        StatusCode::TOO_MANY_REQUESTS,
        [(header::RETRY_AFTER, retry_after_secs.to_string())],
        "too many recent authentication failures",
    )
        .into_response()
}

fn request_client_ip(state: &AppState, headers: &HeaderMap, request: &Request) -> std::net::IpAddr {
    request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map_or(
            std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED),
            |connect_info| {
                resolve_client_ip(
                    &state.shared.config().trusted_proxies,
                    connect_info.0,
                    headers,
                )
            },
        )
}
