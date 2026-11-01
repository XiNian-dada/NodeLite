//! 只读路由 Basic Auth 与 2FA 中间件。

use std::net::SocketAddr;

use axum::Json;
use axum::extract::{ConnectInfo, Request, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::middleware::Next;
use axum::response::{AppendHeaders, IntoResponse, Response};
use serde_json::json;
use tracing::error;

use super::{CurrentLoginEventId, user_agent};
use crate::AppState;
use crate::admission::resolve_client_ip;
use crate::audit::{AuditEventType, NewAuditEvent};
use crate::auth::{
    AuthSessionError, BASIC_AUTH_SESSION_COOKIE, BASIC_AUTH_SESSION_SECS, TWO_FACTOR_AUTH_COOKIE,
    TWO_FACTOR_PENDING_COOKIE, TWO_FACTOR_PENDING_SECS, auth_cookie, cookie_value, expire_cookie,
    secure_cookies,
};
use crate::handlers::record_audit_event;
use crate::ws::{browser_origin_allowed, cross_origin_rejected_response};

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
    // Reject before authentication so foreign-origin handshakes never create browser
    // sessions or login audit records.
    if request.uri().path() == "/ws/browser"
        && is_websocket_upgrade(&headers)
        && !browser_origin_allowed(&headers, &state.shared.config().public_base_url)
    {
        return cross_origin_rejected_response(&headers);
    }
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
                return two_factor_required_response();
            }
            match request.uri().path() {
                // Identical build artifacts for every visitor; the public verify page needs them.
                path if path.starts_with("/assets/") => return next.run(request).await,
                // Scrapers cannot complete 2FA or keep cookies, so they never get a pending
                // session; operators can opt in to Basic-only scraping.
                "/metrics" => {
                    if state.shared.config().metrics.allow_basic_scrape_with_2fa {
                        return next.run(request).await;
                    }
                    return two_factor_required_response();
                }
                _ => {}
            }
            return issue_two_factor_redirect(&state, &headers, request).await;
        }
        // API scrapers do not retain cookies; only UI entry points start browser sessions.
        if !starts_browser_session(&request) {
            return next.run(request).await;
        }
        let has_valid_session = cookie_value(&headers, BASIC_AUTH_SESSION_COOKIE)
            .as_deref()
            .is_some_and(|token| state.two_factor_sessions.is_basic_auth_session_valid(token));
        if !has_valid_session {
            return issue_basic_auth_session_and_continue(&state, &meta, request, next).await;
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

/// 无法跟随 302 的客户端(浏览器 `new WebSocket()`、Prometheus)要求 2FA 时的响应。
/// 返回 401 + JSON 而非重定向;WS 客户端据此跳转 /verify-2fa。
fn two_factor_required_response() -> Response {
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

async fn issue_two_factor_redirect(
    state: &AppState,
    headers: &HeaderMap,
    request: Request,
) -> Response {
    let auth = state.readonly_auth.read().await;
    // 中间件初次校验后可能恰好发生凭据轮换，因此在签发前需在读锁下二次确认 2FA 开启且当前凭据依然有效。
    if !auth.enable_2fa || !auth.is_authorized(&request) {
        return readonly_auth_unauthorized_response();
    }
    // A browser that already holds a live pending session keeps it instead of minting one
    // per request while the verify page loads.
    if cookie_value(headers, TWO_FACTOR_PENDING_COOKIE)
        .is_some_and(|token| state.two_factor_sessions.pending_exists(&token))
    {
        return (
            StatusCode::FOUND,
            [(header::LOCATION, "/verify-2fa".to_string())],
        )
            .into_response();
    }
    let pending_token = match state.two_factor_sessions.create_pending() {
        Ok(token) => token,
        Err(error) => {
            error!(error = ?error, "failed to create pending 2FA session");
            return session_creation_error_response(error);
        }
    };
    drop(auth);
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

/// Node metadata writes reach the agent's traffic rules, so they count as settings writes.
fn is_sensitive_readonly_path(path: &str, request: &Request) -> bool {
    request.method() != axum::http::Method::GET
        && (path.starts_with("/api/settings/") || path.starts_with("/api/nodes/"))
}

fn starts_browser_session(request: &Request) -> bool {
    if request.method() != axum::http::Method::GET {
        return false;
    }
    let path = request.uri().path();
    matches!(path, "/" | "/settings" | "/account" | "/alerts" | "/logs")
        || path.starts_with("/nodes/")
        || (path == "/ws/browser" && is_websocket_upgrade(request.headers()))
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

async fn record_basic_auth_login_success(state: &AppState, meta: &ReadonlyAuthMeta) -> Option<i64> {
    let audit_user = {
        let auth = state.readonly_auth.read().await;
        auth.config.as_ref().map(|config| config.username.clone())
    };
    let geoip_info = state.geoip.lookup(meta.client_ip).await;
    let mut details = json!({
        "path": meta.request_path,
        "basic_auth_only": true,
    });
    if let Some(info) = geoip_info
        && let Some(obj) = details.as_object_mut()
    {
        obj.insert("country".to_string(), json!(info.country));
        if let Some(city) = info.city {
            obj.insert("city".to_string(), json!(city));
        }
        if let Some(lat) = info.latitude {
            obj.insert("latitude".to_string(), json!(lat));
        }
        if let Some(lon) = info.longitude {
            obj.insert("longitude".to_string(), json!(lon));
        }
    }

    let mut event = NewAuditEvent::now(AuditEventType::LoginSuccess, meta.audit_ip.clone(), true);
    event.user = audit_user;
    event.user_agent = meta.audit_user_agent.clone();
    event.details = details;
    match state.audit_log.record_and_return_id(event).await {
        Ok(id) => id,
        Err(error) => {
            error!(error = ?error, "failed to persist basic auth login audit event");
            None
        }
    }
}

async fn issue_basic_auth_session_and_continue(
    state: &AppState,
    meta: &ReadonlyAuthMeta,
    mut request: Request,
    next: Next,
) -> Response {
    // Reserve capacity before recording a login, under the same auth generation check.
    let auth = state.readonly_auth.read().await;
    if auth.enable_2fa || !auth.is_authorized(&request) {
        return readonly_auth_unauthorized_response();
    }
    let session_token = match state.two_factor_sessions.create_basic_auth_session(None) {
        Ok(token) => token,
        Err(error) => {
            error!(error = ?error, "failed to generate basic auth session token");
            return session_creation_error_response(error);
        }
    };
    drop(auth);
    let login_event_id = record_basic_auth_login_success(state, meta).await;
    let Some(lifetime) = state
        .two_factor_sessions
        .bind_basic_auth_login(&session_token, login_event_id)
    else {
        return readonly_auth_unauthorized_response();
    };
    request.extensions_mut().insert(lifetime);
    if let Some(id) = login_event_id {
        request.extensions_mut().insert(CurrentLoginEventId(id));
    }
    let secure = secure_cookies(state.shared.config());
    let mut response = next.run(request).await;
    response.headers_mut().insert(
        header::SET_COOKIE,
        match header::HeaderValue::from_str(
            &auth_cookie(
                BASIC_AUTH_SESSION_COOKIE,
                &session_token,
                BASIC_AUTH_SESSION_SECS,
                secure,
            )
            .1,
        ) {
            Ok(value) => value,
            Err(error) => {
                error!(error = ?error, "failed to format session cookie");
                return StatusCode::INTERNAL_SERVER_ERROR.into_response();
            }
        },
    );
    response
}

fn session_creation_error_response(error: AuthSessionError) -> Response {
    match error {
        AuthSessionError::Capacity => StatusCode::SERVICE_UNAVAILABLE,
        AuthSessionError::Entropy(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
    .into_response()
}

#[cfg(test)]
#[path = "middleware_revocation_tests.rs"]
mod revocation_tests;

#[cfg(test)]
#[path = "basic_session_tests.rs"]
mod basic_session_tests;

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use axum::http::Method;

    use super::*;

    fn request(method: Method, path: &str) -> Request {
        Request::builder()
            .method(method)
            .uri(path)
            .body(Body::empty())
            .expect("request should build")
    }

    #[test]
    fn settings_and_node_writes_are_sensitive_but_reads_are_not() {
        for (method, path, sensitive) in [
            (Method::POST, "/api/settings/password", true),
            (Method::POST, "/api/nodes/edge-01/service-meta", true),
            (Method::POST, "/api/nodes/edge-01/location-override", true),
            (Method::POST, "/api/nodes/edge-01/refresh-token", true),
            (Method::GET, "/api/nodes/edge-01/history", false),
            (Method::GET, "/api/settings", false),
            (Method::POST, "/api/overview", false),
        ] {
            let request = request(method.clone(), path);
            assert_eq!(
                is_sensitive_readonly_path(path, &request),
                sensitive,
                "{method} {path}"
            );
        }
    }
}
