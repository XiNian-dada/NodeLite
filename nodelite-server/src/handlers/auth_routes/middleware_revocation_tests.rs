//! Barriers keep login issuance on the credential-rotation boundary reproducible.

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use axum::response::Response;
use futures::{StreamExt, poll};
use tokio::net::TcpListener;
use tokio::time::timeout;
use tokio_tungstenite::tungstenite::{Message, client::IntoClientRequest};
use tokio_tungstenite::{connect_async, tungstenite};
use tower::ServiceExt;

use super::{evaluate_readonly_auth, issue_two_factor_redirect};
use crate::AppState;
use crate::auth::{
    ReadonlyRouteAuth, TWO_FACTOR_AUTH_COOKIE, TWO_FACTOR_PENDING_COOKIE, cookie_value,
    decode_totp_secret,
};
use crate::test_support::{TEST_BASIC_AUTH_HEADER, TEST_TIMEOUT};
use crate::tests::two_factor_auth_test_state;

#[tokio::test]
async fn pending_issuance_rechecks_credentials_and_two_factor_mode() {
    for disable in [false, true] {
        let (state, temp_dir) = two_factor_auth_test_state("pending-rotation", true).await;
        let request = Request::builder()
            .uri("/api/bootstrap")
            .header(header::AUTHORIZATION, TEST_BASIC_AUTH_HEADER)
            .body(Body::empty())
            .expect("request");
        let mut auth = state.readonly_auth.write().await;
        assert_eq!(
            evaluate_readonly_auth(&state, &auth, request.headers(), &request),
            Some((true, true)),
        );
        rotate_credentials(&state, &mut auth, disable);
        drop(auth);

        let response = issue_two_factor_redirect(&state, request).await;
        let status = response.status();
        let cookie = response_cookie(&response, TWO_FACTOR_PENDING_COOKIE);
        cleanup(state, temp_dir).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert!(cookie.is_none(), "stale Basic auth issued a pending cookie");
    }
}

#[tokio::test]
async fn in_flight_verification_cannot_create_a_session_after_password_rotation() {
    let (state, temp_dir) = two_factor_auth_test_state("verify-rotation", true).await;
    let app = crate::startup::build_router(state.clone());
    let pending = state.two_factor_sessions.create_pending().expect("pending");
    let mut auth = state.readonly_auth.write().await;
    let code = totp_code(&auth, 0);
    let mut response = Box::pin(app.clone().oneshot(verify_request(&pending, &code)));
    assert!(poll!(response.as_mut()).is_pending());
    rotate_credentials(&state, &mut auth, false);
    assert!(!state.two_factor_sessions.pending_exists(&pending));
    drop(auth);

    let response = timeout(TEST_TIMEOUT, response)
        .await
        .expect("verification unblocks")
        .expect("response");
    let cookie = response_cookie(&response, TWO_FACTOR_AUTH_COOKIE).unwrap_or_default();
    let bootstrap = bootstrap_status(&app, &cookie).await;
    let websocket = browser_status(&state, &cookie).await;
    cleanup(state, temp_dir).await;
    assert_eq!(
        bootstrap,
        StatusCode::UNAUTHORIZED,
        "revoked login authenticated HTTP"
    );
    assert_eq!(
        websocket,
        StatusCode::UNAUTHORIZED,
        "revoked login authenticated WS"
    );
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn concurrent_verifications_can_exchange_a_pending_session_only_once() {
    let (state, temp_dir) = two_factor_auth_test_state("verify-once", true).await;
    let app = crate::startup::build_router(state.clone());
    let pending = state.two_factor_sessions.create_pending().expect("pending");
    let auth = state.readonly_auth.write().await;
    let mut first = Box::pin(
        app.clone()
            .oneshot(verify_request(&pending, &totp_code(&auth, 0))),
    );
    let mut second = Box::pin(
        app.clone()
            .oneshot(verify_request(&pending, &totp_code(&auth, 30))),
    );
    assert!(poll!(first.as_mut()).is_pending());
    assert!(poll!(second.as_mut()).is_pending());
    drop(auth);

    let (first, second) = timeout(TEST_TIMEOUT, async { tokio::join!(first, second) })
        .await
        .expect("verification unblocks");
    let responses = [
        first.expect("first response"),
        second.expect("second response"),
    ];
    let successes = responses
        .iter()
        .filter(|response| response.status() == StatusCode::OK)
        .count();
    cleanup(state, temp_dir).await;
    assert_eq!(successes, 1, "one pending cookie issued multiple sessions");
}

#[tokio::test]
async fn a_fresh_login_after_password_rotation_authenticates_http_and_websocket() {
    let (state, temp_dir) = two_factor_auth_test_state("verify-new", true).await;
    let app = crate::startup::build_router(state.clone());
    let mut auth = state.readonly_auth.write().await;
    rotate_credentials(&state, &mut auth, false);
    let header = auth.expected_authorization.clone().expect("Basic header");
    let code = totp_code(&auth, 0);
    drop(auth);
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/bootstrap")
                .header(header::AUTHORIZATION, header)
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("pending response");
    assert_eq!(response.status(), StatusCode::FOUND);
    let pending_cookie =
        response_cookie(&response, TWO_FACTOR_PENDING_COOKIE).expect("pending cookie");
    let pending = pending_cookie.split_once('=').expect("cookie separator").1;
    let response = app
        .clone()
        .oneshot(verify_request(pending, &code))
        .await
        .expect("login response");
    assert_eq!(response.status(), StatusCode::OK);
    let cookie = response_cookie(&response, TWO_FACTOR_AUTH_COOKIE).expect("authenticated cookie");
    assert_eq!(bootstrap_status(&app, &cookie).await, StatusCode::OK);
    assert_eq!(
        browser_status(&state, &cookie).await,
        StatusCode::SWITCHING_PROTOCOLS
    );
    cleanup(state, temp_dir).await;
}

fn rotate_credentials(state: &AppState, auth: &mut ReadonlyRouteAuth, disable: bool) {
    let mut config = auth.config.clone().expect("configured auth");
    if disable {
        config.enable_2fa = false;
        config.totp_secret = None;
    } else {
        config.password = "NewStrongPassword42!".to_string();
    }
    auth.revoked.cancel();
    *auth = ReadonlyRouteAuth::from_config(Some(config));
    state.two_factor_sessions.clear_authenticated();
}

fn totp_code(auth: &ReadonlyRouteAuth, offset_secs: u64) -> String {
    let secret = auth
        .config
        .as_ref()
        .and_then(|auth| auth.totp_secret.as_deref())
        .and_then(decode_totp_secret)
        .expect("TOTP secret");
    totp_lite::totp_custom::<totp_lite::Sha1>(
        30,
        6,
        &secret,
        chrono::Utc::now().timestamp() as u64 + offset_secs,
    )
}

fn verify_request(pending: &str, code: &str) -> Request<Body> {
    let mut request = Request::builder()
        .method("POST")
        .uri("/api/verify-2fa")
        .header(
            header::COOKIE,
            format!("{TWO_FACTOR_PENDING_COOKIE}={pending}"),
        )
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(serde_json::json!({ "code": code }).to_string()))
        .expect("request");
    request.extensions_mut().insert(axum::extract::ConnectInfo(
        "127.0.0.1:50001"
            .parse::<std::net::SocketAddr>()
            .expect("peer"),
    ));
    request
}

fn response_cookie(response: &Response, name: &str) -> Option<String> {
    response
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .find_map(|header| {
            let mut headers = axum::http::HeaderMap::new();
            headers.insert(header::COOKIE, header.clone());
            cookie_value(&headers, name).map(|value| format!("{name}={value}"))
        })
}

async fn bootstrap_status(app: &Router, cookie: &str) -> StatusCode {
    app.clone()
        .oneshot(
            Request::builder()
                .uri("/api/bootstrap")
                .header(header::COOKIE, cookie)
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("bootstrap response")
        .status()
}

async fn browser_status(state: &AppState, cookie: &str) -> StatusCode {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let addr = listener.local_addr().expect("address");
    let app = crate::startup::build_router(state.clone());
    let shutdown = tokio_util::sync::CancellationToken::new();
    let stop = shutdown.clone();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .with_graceful_shutdown(stop.cancelled_owned())
        .await
        .expect("server");
    });
    let mut request = format!("ws://{addr}/ws/browser")
        .into_client_request()
        .expect("WS request");
    request
        .headers_mut()
        .insert(header::COOKIE, cookie.parse().expect("cookie header"));
    let status = match timeout(TEST_TIMEOUT, connect_async(request))
        .await
        .expect("WS handshake")
    {
        Ok((mut socket, response)) => {
            let message = timeout(TEST_TIMEOUT, socket.next())
                .await
                .expect("initial state");
            assert!(
                matches!(message, Some(Ok(Message::Text(text))) if text.contains("initial_state"))
            );
            socket.close(None).await.expect("close websocket");
            response.status()
        }
        Err(tungstenite::Error::Http(response)) => response.status(),
        Err(error) => panic!("unexpected websocket failure: {error}"),
    };
    shutdown.cancel();
    timeout(TEST_TIMEOUT, server)
        .await
        .expect("server shutdown")
        .expect("server task");
    status
}

async fn cleanup(state: AppState, temp_dir: std::path::PathBuf) {
    state.history.shutdown().await;
    state.audit_log.shutdown().await;
    std::fs::remove_dir_all(temp_dir).expect("cleanup");
}
