//! Machine requests must not allocate browser sessions or manufacture login history.

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use axum::middleware::from_fn_with_state;
use axum::routing::get;
use tower::ServiceExt;

use super::{require_readonly_auth, starts_browser_session};
use crate::AppState;
use crate::audit::{AuditEvent, AuditEventType, AuditQuery};
use crate::auth::{BASIC_AUTH_SESSION_COOKIE, MAX_AUTH_SESSIONS, cookie_value};
use crate::test_support::TEST_BASIC_AUTH_HEADER;
use crate::tests::two_factor_auth_test_state;

fn request(path: &str, cookie: Option<&str>) -> Request<Body> {
    let mut builder = Request::builder()
        .uri(path)
        .header(header::AUTHORIZATION, TEST_BASIC_AUTH_HEADER);
    if let Some(cookie) = cookie {
        builder = builder.header(header::COOKIE, cookie);
    }
    builder.body(Body::empty()).expect("request")
}

fn app(state: &AppState) -> Router {
    Router::new()
        .route("/", get(|| async { StatusCode::OK }))
        .route("/metrics", get(|| async { StatusCode::OK }))
        .route("/api/overview", get(|| async { StatusCode::OK }))
        .route("/assets/test.js", get(|| async { StatusCode::OK }))
        .route_layer(from_fn_with_state(state.clone(), require_readonly_auth))
}

async fn logins(state: &AppState) -> Vec<AuditEvent> {
    state
        .audit_log
        .query(AuditQuery {
            start: None,
            end: None,
            event_type: Some(AuditEventType::LoginSuccess),
            success: Some(true),
            limit: 100,
        })
        .await
        .expect("login audit query")
}

async fn cleanup(state: AppState, temp_dir: std::path::PathBuf) {
    state.history.shutdown().await;
    state.audit_log.shutdown().await;
    std::fs::remove_dir_all(temp_dir).expect("cleanup");
}

#[test]
fn only_ui_get_requests_establish_browser_sessions() {
    for path in [
        "/",
        "/settings",
        "/account",
        "/alerts",
        "/logs",
        "/nodes/example",
    ] {
        let mut request = request(path, None);
        assert!(starts_browser_session(&request), "UI entry: {path}");
        *request.method_mut() = axum::http::Method::POST;
        assert!(!starts_browser_session(&request), "non-navigation: {path}");
    }
    for path in [
        "/api/overview",
        "/api/auth/last-login",
        "/assets/test.js",
        "/metrics",
        "/ws/browser",
    ] {
        assert!(
            !starts_browser_session(&request(path, None)),
            "not a UI entry: {path}"
        );
    }
    let mut ws = request("/ws/browser", None);
    ws.headers_mut().insert(
        header::UPGRADE,
        "websocket".parse().expect("upgrade header"),
    );
    assert!(starts_browser_session(&ws));
}

#[tokio::test]
async fn cookieless_machine_requests_do_not_create_sessions_or_login_events() {
    let (state, temp_dir) = two_factor_auth_test_state("stateless-basic", false).await;
    let app = app(&state);
    for index in 0..2_000 {
        let path = ["/metrics", "/api/overview", "/assets/test.js"][index % 3];
        let cookie = (index % 2 == 0).then_some("nodelite_basic_session=invalid");
        let mut request = request(path, cookie);
        request
            .headers_mut()
            .insert(header::ACCEPT, "text/html".parse().expect("accept header"));
        let response = app.clone().oneshot(request).await.expect("response");
        assert_eq!(response.status(), StatusCode::OK);
        assert!(response.headers().get(header::SET_COOKIE).is_none());
    }
    assert!(logins(&state).await.is_empty());
    cleanup(state, temp_dir).await;
}

#[tokio::test]
async fn browser_navigation_keeps_one_login_event_for_cookie_backed_requests() {
    let (state, temp_dir) = two_factor_auth_test_state("browser-basic", false).await;
    let app = app(&state);
    let response = app
        .clone()
        .oneshot(request("/", None))
        .await
        .expect("navigation");
    assert_eq!(response.status(), StatusCode::OK);
    let cookie = response.headers()[header::SET_COOKIE]
        .to_str()
        .expect("cookie")
        .split(';')
        .next()
        .expect("cookie pair")
        .to_string();
    let mut headers = axum::http::HeaderMap::new();
    headers.insert(header::COOKIE, cookie.parse().expect("cookie header"));
    let token = cookie_value(&headers, BASIC_AUTH_SESSION_COOKIE).expect("session token");
    assert!(
        state
            .two_factor_sessions
            .is_basic_auth_session_valid(&token)
    );
    let events = logins(&state).await;
    assert_eq!(events.len(), 1);
    assert_eq!(
        state
            .two_factor_sessions
            .get_basic_auth_login_event_id(&token),
        Some(events[0].id)
    );
    for path in ["/api/overview", "/metrics", "/"] {
        let response = app
            .clone()
            .oneshot(request(path, Some(&cookie)))
            .await
            .expect("repeat request");
        assert_eq!(response.status(), StatusCode::OK);
        assert!(response.headers().get(header::SET_COOKIE).is_none());
    }
    assert_eq!(logins(&state).await.len(), 1);
    state.two_factor_sessions.clear_authenticated();
    assert!(
        !state
            .two_factor_sessions
            .is_basic_auth_session_valid(&token)
    );
    cleanup(state, temp_dir).await;
}

#[tokio::test]
async fn full_session_store_rejects_new_browsers_without_logging_success_or_blocking_metrics() {
    let (state, temp_dir) = two_factor_auth_test_state("full-basic", false).await;
    let app = app(&state);
    let token = state
        .two_factor_sessions
        .create_basic_auth_session(None)
        .expect("basic");
    for _ in 1..MAX_AUTH_SESSIONS {
        state
            .two_factor_sessions
            .create_pending()
            .expect("fill store");
    }
    let response = app
        .clone()
        .oneshot(request("/", None))
        .await
        .expect("new browser");
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert!(response.headers().get(header::SET_COOKIE).is_none());
    assert!(logins(&state).await.is_empty());
    let cookie = format!("{BASIC_AUTH_SESSION_COOKIE}={token}");
    for (path, cookie) in [
        ("/", Some(cookie.as_str())),
        ("/metrics", None),
        ("/api/overview", None),
    ] {
        let response = app
            .clone()
            .oneshot(request(path, cookie))
            .await
            .expect("existing client");
        assert_eq!(response.status(), StatusCode::OK);
        assert!(response.headers().get(header::SET_COOKIE).is_none());
    }
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/metrics")
                .body(Body::empty())
                .expect("unauthorized request"),
        )
        .await
        .expect("unauthorized");
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert!(
        state
            .two_factor_sessions
            .is_basic_auth_session_valid(&token)
    );
    assert!(logins(&state).await.is_empty());
    cleanup(state, temp_dir).await;
}

#[tokio::test]
async fn full_pending_store_cannot_bypass_two_factor_authentication() {
    let (state, temp_dir) = two_factor_auth_test_state("full-pending", true).await;
    let app = app(&state);
    for _ in 0..MAX_AUTH_SESSIONS {
        state
            .two_factor_sessions
            .create_pending()
            .expect("fill store");
    }
    let response = app
        .oneshot(request("/api/overview", None))
        .await
        .expect("2FA response");
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert!(response.headers().get(header::SET_COOKIE).is_none());
    assert!(logins(&state).await.is_empty());
    cleanup(state, temp_dir).await;
}
