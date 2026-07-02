//! Readonly-auth, 2FA, and audit-route tests.

use std::net::{IpAddr, Ipv4Addr, SocketAddr, SocketAddrV4};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{HeaderMap, Request, StatusCode, header};
use axum::middleware::from_fn_with_state;
use axum::routing::{get, post};
use chrono::{TimeZone, Utc};
use serde_json::json;
use tokio::runtime::Runtime;
use tower::util::ServiceExt;

use super::AppState;
use super::support::{
    protected_ok, protected_request, two_factor_auth_test_state, ws_upgrade_request,
};
use crate::audit::{AuditEvent, AuditEventType, AuditQuery, NewAuditEvent};
use crate::auth::{BASIC_AUTH_SESSION_COOKIE, TWO_FACTOR_AUTH_COOKIE};
use crate::handlers::{audit_log, last_login, require_readonly_auth};
use crate::test_support::{TEST_BASIC_AUTH_HEADER, test_server_config, test_ws_config};

#[test]
fn readonly_auth_route_accepts_valid_basic_auth() {
    let runtime = Runtime::new().expect("runtime should build");
    runtime.block_on(async {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be monotonic enough")
            .as_nanos();
        let temp_dir = std::env::temp_dir().join(format!("nodelite-readonly-auth-ok-{unique}"));
        std::fs::create_dir_all(&temp_dir).expect("temp dir should exist");
        let registry_path = temp_dir.join("server.json");
        let config = test_server_config(
            SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 8080)),
            "https://monitor.example.com".to_string(),
            registry_path,
            temp_dir.join("history.sqlite3"),
            temp_dir.join("snapshot.json"),
        );
        let state = AppState::test_fixture(config.into(), Arc::new(temp_dir.join("server.toml")))
            .await
            .expect("state fixture should build");
        let app: Router = Router::new()
            .route("/api/overview", get(protected_ok))
            .route_layer(from_fn_with_state(state.clone(), require_readonly_auth))
            .with_state(state);
        let response = app
            .oneshot(protected_request(
                "GET",
                "/api/overview",
                Some(TEST_BASIC_AUTH_HEADER),
                SocketAddr::V4(SocketAddrV4::new(
                    "198.51.100.24".parse().expect("ip"),
                    51234,
                )),
            ))
            .await
            .expect("response should be produced");

        assert_eq!(response.status(), StatusCode::OK);
        let _ = std::fs::remove_dir_all(&temp_dir);
    });
}

#[test]
fn last_login_uses_session_event_id_when_login_timestamps_match() {
    let runtime = Runtime::new().expect("runtime should build");
    runtime.block_on(async {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be monotonic enough")
            .as_nanos();
        let temp_dir = std::env::temp_dir().join(format!("nodelite-last-login-id-{unique}"));
        std::fs::create_dir_all(&temp_dir).expect("temp dir should exist");
        let registry_path = temp_dir.join("server.json");
        let config = test_server_config(
            SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 8080)),
            "https://monitor.example.com".to_string(),
            registry_path,
            temp_dir.join("history.sqlite3"),
            temp_dir.join("snapshot.json"),
        );
        let state = AppState::test_fixture(config.into(), Arc::new(temp_dir.join("server.toml")))
            .await
            .expect("state fixture should build");
        let timestamp = Utc
            .timestamp_opt(1_700_000_000, 0)
            .single()
            .expect("fixed timestamp should be valid");

        let mut previous_login =
            NewAuditEvent::now(AuditEventType::LoginSuccess, "198.51.100.24", true);
        previous_login.timestamp = timestamp;
        previous_login.user_agent = Some("CodexReview/previous".to_string());
        state
            .audit_log
            .record_and_return_id(previous_login)
            .await
            .expect("previous login should persist")
            .expect("previous login id should be returned");

        let mut current_login =
            NewAuditEvent::now(AuditEventType::LoginSuccess, "198.51.100.24", true);
        current_login.timestamp = timestamp;
        current_login.user_agent = Some("CodexReview/current".to_string());
        let current_id = state
            .audit_log
            .record_and_return_id(current_login)
            .await
            .expect("current login should persist")
            .expect("current login id should be returned");
        let session_token = state
            .two_factor_sessions
            .create_basic_auth_session(Some(current_id))
            .expect("session token should be created");

        let mut headers = HeaderMap::new();
        headers.insert(
            header::COOKIE,
            format!("{BASIC_AUTH_SESSION_COOKIE}={session_token}")
                .parse()
                .expect("cookie header should parse"),
        );

        let info = last_login(axum::extract::State(state.clone()), headers)
            .await
            .expect("last-login response should succeed")
            .0;
        let expected_timestamp = timestamp.to_rfc3339();
        assert_eq!(info.user_agent.as_deref(), Some("CodexReview/previous"));
        assert_eq!(info.timestamp.as_deref(), Some(expected_timestamp.as_str()));

        let _ = std::fs::remove_dir_all(&temp_dir);
    });
}

#[test]
fn last_login_uses_two_factor_session_event_id_when_login_timestamps_match() {
    let runtime = Runtime::new().expect("runtime should build");
    runtime.block_on(async {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be monotonic enough")
            .as_nanos();
        let temp_dir = std::env::temp_dir().join(format!("nodelite-last-login-2fa-id-{unique}"));
        std::fs::create_dir_all(&temp_dir).expect("temp dir should exist");
        let registry_path = temp_dir.join("server.json");
        let config = test_server_config(
            SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 8080)),
            "https://monitor.example.com".to_string(),
            registry_path,
            temp_dir.join("history.sqlite3"),
            temp_dir.join("snapshot.json"),
        );
        let state = AppState::test_fixture(config.into(), Arc::new(temp_dir.join("server.toml")))
            .await
            .expect("state fixture should build");
        let timestamp = Utc
            .timestamp_opt(1_700_000_000, 0)
            .single()
            .expect("fixed timestamp should be valid");

        let mut previous_login =
            NewAuditEvent::now(AuditEventType::LoginSuccess, "198.51.100.24", true);
        previous_login.timestamp = timestamp;
        previous_login.user_agent = Some("CodexReview/previous-2fa".to_string());
        state
            .audit_log
            .record_and_return_id(previous_login)
            .await
            .expect("previous login should persist")
            .expect("previous login id should be returned");

        let mut current_login =
            NewAuditEvent::now(AuditEventType::LoginSuccess, "198.51.100.24", true);
        current_login.timestamp = timestamp;
        current_login.user_agent = Some("CodexReview/current-2fa".to_string());
        let current_id = state
            .audit_log
            .record_and_return_id(current_login)
            .await
            .expect("current login should persist")
            .expect("current login id should be returned");
        let session_token = state
            .two_factor_sessions
            .create_authenticated()
            .expect("session token should be created");
        state
            .two_factor_sessions
            .set_authenticated_login_event_id(&session_token, Some(current_id));

        let mut headers = HeaderMap::new();
        headers.insert(
            header::COOKIE,
            format!("{TWO_FACTOR_AUTH_COOKIE}={session_token}")
                .parse()
                .expect("cookie header should parse"),
        );

        let info = last_login(axum::extract::State(state.clone()), headers)
            .await
            .expect("last-login response should succeed")
            .0;
        let expected_timestamp = timestamp.to_rfc3339();
        assert_eq!(info.user_agent.as_deref(), Some("CodexReview/previous-2fa"));
        assert_eq!(info.timestamp.as_deref(), Some(expected_timestamp.as_str()));

        let _ = std::fs::remove_dir_all(&temp_dir);
    });
}

#[test]
fn websocket_upgrade_requiring_two_factor_returns_401_json_not_redirect() {
    let runtime = Runtime::new().expect("runtime should build");
    runtime.block_on(async {
        let (state, temp_dir) = two_factor_auth_test_state("ws-2fa-401", true).await;
        let app: Router = Router::new()
            .route("/ws/browser", get(protected_ok))
            .route_layer(from_fn_with_state(state.clone(), require_readonly_auth))
            .with_state(state);
        let response = app
            .oneshot(ws_upgrade_request(
                "/ws/browser",
                Some(TEST_BASIC_AUTH_HEADER),
                SocketAddr::V4(SocketAddrV4::new(
                    "198.51.100.24".parse().expect("ip"),
                    51234,
                )),
            ))
            .await
            .expect("response should be produced");

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert!(response.headers().get(header::LOCATION).is_none());
        let body = to_bytes(response.into_body(), 64 * 1024)
            .await
            .expect("body should read");
        let json: serde_json::Value = serde_json::from_slice(&body).expect("json body");
        assert_eq!(json["ok"], serde_json::Value::Bool(false));
        assert_eq!(json["message"], "two_factor_required");
        assert_eq!(json["endpoint"], "/verify-2fa");
        let _ = std::fs::remove_dir_all(&temp_dir);
    });
}

#[test]
fn http_request_requiring_two_factor_still_redirects() {
    let runtime = Runtime::new().expect("runtime should build");
    runtime.block_on(async {
        let (state, temp_dir) = two_factor_auth_test_state("http-2fa-302", true).await;
        let app: Router = Router::new()
            .route("/api/overview", get(protected_ok))
            .route_layer(from_fn_with_state(state.clone(), require_readonly_auth))
            .with_state(state);
        let response = app
            .oneshot(protected_request(
                "GET",
                "/api/overview",
                Some(TEST_BASIC_AUTH_HEADER),
                SocketAddr::V4(SocketAddrV4::new(
                    "198.51.100.24".parse().expect("ip"),
                    51234,
                )),
            ))
            .await
            .expect("response should be produced");

        assert_eq!(response.status(), StatusCode::FOUND);
        assert_eq!(
            response
                .headers()
                .get(header::LOCATION)
                .expect("location header"),
            "/verify-2fa"
        );
        let _ = std::fs::remove_dir_all(&temp_dir);
    });
}

#[test]
fn websocket_upgrade_without_two_factor_passes_through() {
    let runtime = Runtime::new().expect("runtime should build");
    runtime.block_on(async {
        let (state, temp_dir) = two_factor_auth_test_state("ws-no-2fa-pass", false).await;
        let app: Router = Router::new()
            .route("/ws/browser", get(protected_ok))
            .route_layer(from_fn_with_state(state.clone(), require_readonly_auth))
            .with_state(state);
        let response = app
            .oneshot(ws_upgrade_request(
                "/ws/browser",
                Some(TEST_BASIC_AUTH_HEADER),
                SocketAddr::V4(SocketAddrV4::new(
                    "198.51.100.24".parse().expect("ip"),
                    51234,
                )),
            ))
            .await
            .expect("response should be produced");

        assert_eq!(response.status(), StatusCode::OK);
        let _ = std::fs::remove_dir_all(&temp_dir);
    });
}

#[test]
fn readonly_auth_route_logs_missing_basic_auth_reason() {
    let runtime = Runtime::new().expect("runtime should build");
    runtime.block_on(async {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be monotonic enough")
            .as_nanos();
        let temp_dir =
            std::env::temp_dir().join(format!("nodelite-readonly-auth-missing-{unique}"));
        std::fs::create_dir_all(&temp_dir).expect("temp dir should exist");
        let registry_path = temp_dir.join("server.json");
        let config = test_server_config(
            SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 8080)),
            "https://monitor.example.com".to_string(),
            registry_path,
            temp_dir.join("history.sqlite3"),
            temp_dir.join("snapshot.json"),
        );
        let state = AppState::test_fixture(config.into(), Arc::new(temp_dir.join("server.toml")))
            .await
            .expect("state fixture should build");
        let app: Router = Router::new()
            .route("/api/overview", get(protected_ok))
            .route_layer(from_fn_with_state(state.clone(), require_readonly_auth))
            .with_state(state.clone());
        let response = app
            .oneshot(protected_request(
                "GET",
                "/api/overview",
                None,
                SocketAddr::V4(SocketAddrV4::new(
                    "198.51.100.24".parse().expect("ip"),
                    51234,
                )),
            ))
            .await
            .expect("response should be produced");

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        let events = state
            .audit_log
            .query(AuditQuery {
                start: None,
                end: None,
                event_type: Some(AuditEventType::LoginFailure),
                success: Some(false),
                limit: 4,
            })
            .await
            .expect("audit query should succeed");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].details["reason"], "missing_basic_auth");
        let _ = std::fs::remove_dir_all(&temp_dir);
    });
}

#[test]
fn readonly_auth_route_blocks_after_repeated_invalid_credentials() {
    let runtime = Runtime::new().expect("runtime should build");
    runtime.block_on(async {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be monotonic enough")
            .as_nanos();
        let temp_dir = std::env::temp_dir().join(format!("nodelite-readonly-auth-block-{unique}"));
        std::fs::create_dir_all(&temp_dir).expect("temp dir should exist");
        let registry_path = temp_dir.join("server.json");
        let mut config = test_server_config(
            SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 8080)),
            "https://monitor.example.com".to_string(),
            registry_path,
            temp_dir.join("history.sqlite3"),
            temp_dir.join("snapshot.json"),
        );
        config.ws.auth_fail_max_attempts = 2;
        config.ws.auth_block_secs = 1;
        let state = AppState::test_fixture(config.into(), Arc::new(temp_dir.join("server.toml")))
            .await
            .expect("state fixture should build");
        let app: Router = Router::new()
            .route("/api/overview", get(protected_ok))
            .route_layer(from_fn_with_state(state.clone(), require_readonly_auth))
            .with_state(state.clone());
        let peer_addr = SocketAddr::V4(SocketAddrV4::new(
            "198.51.100.24".parse().expect("ip"),
            51234,
        ));

        for _ in 0..2 {
            let response = app
                .clone()
                .oneshot(protected_request(
                    "GET",
                    "/api/overview",
                    Some("Basic Zm9vOmJhcg=="),
                    peer_addr,
                ))
                .await
                .expect("response should be produced");
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        }

        let blocked = app
            .oneshot(protected_request(
                "GET",
                "/api/overview",
                Some("Basic Zm9vOmJhcg=="),
                peer_addr,
            ))
            .await
            .expect("response should be produced");
        assert_eq!(blocked.status(), StatusCode::TOO_MANY_REQUESTS);
        assert!(blocked.headers().contains_key(header::RETRY_AFTER));

        let events = state
            .audit_log
            .query(AuditQuery {
                start: None,
                end: None,
                event_type: None,
                success: Some(false),
                limit: 8,
            })
            .await
            .expect("audit query should succeed");
        assert!(
            events
                .iter()
                .any(|event| event.event_type == AuditEventType::LoginFailure
                    && event.details["reason"] == "invalid_basic_auth")
        );
        assert!(events.iter().any(
            |event| event.event_type == AuditEventType::RateLimitExceeded
                && event.details["reason"] == "readonly_auth_block"
        ));

        let _ = std::fs::remove_dir_all(&temp_dir);
    });
}

#[test]
fn sensitive_readonly_routes_use_stricter_budget_and_unblock_after_window() {
    let runtime = Runtime::new().expect("runtime should build");
    runtime.block_on(async {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be monotonic enough")
            .as_nanos();
        let temp_dir =
            std::env::temp_dir().join(format!("nodelite-sensitive-readonly-auth-block-{unique}"));
        std::fs::create_dir_all(&temp_dir).expect("temp dir should exist");
        let registry_path = temp_dir.join("server.json");
        let mut config = test_server_config(
            SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 8080)),
            "https://monitor.example.com".to_string(),
            registry_path,
            temp_dir.join("history.sqlite3"),
            temp_dir.join("snapshot.json"),
        );
        config.ws.auth_fail_max_attempts = 4;
        config.ws.auth_block_secs = 1;
        let state = AppState::test_fixture(config.into(), Arc::new(temp_dir.join("server.toml")))
            .await
            .expect("state fixture should build");
        let app: Router = Router::new()
            .route("/api/settings/password", post(protected_ok))
            .route_layer(from_fn_with_state(state.clone(), require_readonly_auth))
            .with_state(state);
        let peer_addr = SocketAddr::V4(SocketAddrV4::new(
            "198.51.100.24".parse().expect("ip"),
            51234,
        ));

        for _ in 0..2 {
            let response = app
                .clone()
                .oneshot(protected_request(
                    "POST",
                    "/api/settings/password",
                    Some("Basic Zm9vOmJhcg=="),
                    peer_addr,
                ))
                .await
                .expect("response should be produced");
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        }

        let blocked = app
            .clone()
            .oneshot(protected_request(
                "POST",
                "/api/settings/password",
                Some("Basic Zm9vOmJhcg=="),
                peer_addr,
            ))
            .await
            .expect("response should be produced");
        assert_eq!(blocked.status(), StatusCode::TOO_MANY_REQUESTS);

        tokio::time::sleep(std::time::Duration::from_millis(1100)).await;

        let unblocked = app
            .oneshot(protected_request(
                "POST",
                "/api/settings/password",
                Some(TEST_BASIC_AUTH_HEADER),
                peer_addr,
            ))
            .await
            .expect("response should be produced");
        assert_eq!(unblocked.status(), StatusCode::OK);

        let _ = std::fs::remove_dir_all(&temp_dir);
    });
}

#[test]
fn audit_log_route_returns_recent_filtered_events() {
    let runtime = Runtime::new().expect("runtime should build");
    runtime.block_on(async {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be monotonic enough")
            .as_nanos();
        let temp_dir = std::env::temp_dir().join(format!("nodelite-audit-route-test-{unique}"));
        std::fs::create_dir_all(&temp_dir).expect("temp dir should exist");
        let registry_path = temp_dir.join("server.json");
        let mut config = test_server_config(
            SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 8080)),
            "https://monitor.example.com".to_string(),
            registry_path,
            temp_dir.join("history.sqlite3"),
            temp_dir.join("snapshot.json"),
        );
        config.readonly_auth = None;
        config.ws = test_ws_config(32, 8);
        config.stale_after_secs = 20;
        config.ping_interval_secs = 10;
        config.ignored_filesystems = Vec::new();
        let state = AppState::test_fixture(config.into(), Arc::new(temp_dir.join("server.toml")))
            .await
            .expect("state fixture should build");
        let mut event = NewAuditEvent::now(
            AuditEventType::LoginFailure,
            IpAddr::V4(Ipv4Addr::LOCALHOST).to_string(),
            false,
        );
        event.user = Some("viewer".to_string());
        event.details = json!({
            "reason": "invalid_credentials",
            "method": "basic_auth",
        });
        state
            .audit_log
            .record(event)
            .await
            .expect("audit event should persist");
        let app: Router = Router::new()
            .route("/api/audit-log", get(audit_log))
            .route_layer(from_fn_with_state(state.clone(), require_readonly_auth))
            .with_state(state);
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/audit-log?event_type=login_failure&success=false&limit=1")
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("response should be produced");

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body should be readable");
        let events: Vec<AuditEvent> =
            serde_json::from_slice(&body).expect("audit payload should be json");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, AuditEventType::LoginFailure);
        assert_eq!(events[0].user.as_deref(), Some("viewer"));
        assert!(!events[0].success);

        let _ = std::fs::remove_dir_all(&temp_dir);
    });
}

#[test]
fn audit_log_route_rejects_unknown_event_type() {
    let runtime = Runtime::new().expect("runtime should build");
    runtime.block_on(async {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be monotonic enough")
            .as_nanos();
        let temp_dir =
            std::env::temp_dir().join(format!("nodelite-audit-route-invalid-test-{unique}"));
        std::fs::create_dir_all(&temp_dir).expect("temp dir should exist");
        let registry_path = temp_dir.join("server.json");
        let mut config = test_server_config(
            SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 8080)),
            "https://monitor.example.com".to_string(),
            registry_path,
            temp_dir.join("history.sqlite3"),
            temp_dir.join("snapshot.json"),
        );
        config.readonly_auth = None;
        config.ws = test_ws_config(32, 8);
        config.stale_after_secs = 20;
        config.ping_interval_secs = 10;
        config.ignored_filesystems = Vec::new();
        let state = AppState::test_fixture(config.into(), Arc::new(temp_dir.join("server.toml")))
            .await
            .expect("state fixture should build");
        let app: Router = Router::new()
            .route("/api/audit-log", get(audit_log))
            .route_layer(from_fn_with_state(state.clone(), require_readonly_auth))
            .with_state(state);
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/audit-log?event_type=nope")
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("response should be produced");

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        let _ = std::fs::remove_dir_all(&temp_dir);
    });
}
