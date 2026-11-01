//! Requests that cannot finish 2FA must not mint pending sessions (#409).

use std::net::{SocketAddr, SocketAddrV4};

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use axum::middleware::from_fn_with_state;
use axum::routing::get;
use tower::util::ServiceExt;

use super::AppState;
use super::support::{protected_ok, two_factor_auth_test_state, two_factor_auth_test_state_with};
use crate::auth::TWO_FACTOR_PENDING_COOKIE;
use crate::handlers::require_readonly_auth;
use crate::test_support::TEST_BASIC_AUTH_HEADER;

fn app(state: &AppState) -> Router {
    Router::new()
        .route("/api/overview", get(protected_ok))
        .route("/metrics", get(protected_ok))
        .route("/assets/{*path}", get(protected_ok))
        .route_layer(from_fn_with_state(state.clone(), require_readonly_auth))
        .with_state(state.clone())
}

fn request(path: &str, cookie: Option<&str>) -> Request<Body> {
    let mut builder = Request::builder()
        .uri(path)
        .header(header::AUTHORIZATION, TEST_BASIC_AUTH_HEADER);
    if let Some(cookie) = cookie {
        builder = builder.header(header::COOKIE, cookie);
    }
    let mut request = builder.body(Body::empty()).expect("request should build");
    request
        .extensions_mut()
        .insert(axum::extract::ConnectInfo(SocketAddr::V4(
            SocketAddrV4::new("198.51.100.40".parse().expect("ip"), 51234),
        )));
    request
}

fn pending_cookie(response: &axum::response::Response) -> Option<String> {
    response
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .find(|value| value.starts_with(&format!("{TWO_FACTOR_PENDING_COOKIE}=")))
        .and_then(|value| value.split(';').next())
        .map(str::to_string)
}

#[tokio::test]
async fn assets_and_metrics_do_not_create_pending_sessions() {
    let (state, temp_dir) = two_factor_auth_test_state("pending-assets", true).await;
    let assets = app(&state)
        .oneshot(request("/assets/brand-icon-light.svg", None))
        .await
        .expect("asset response");
    assert_eq!(assets.status(), StatusCode::OK);
    assert!(pending_cookie(&assets).is_none());

    let metrics = app(&state)
        .oneshot(request("/metrics", None))
        .await
        .expect("metrics response");
    assert_eq!(metrics.status(), StatusCode::UNAUTHORIZED);
    assert!(metrics.headers().get(header::LOCATION).is_none());
    assert!(pending_cookie(&metrics).is_none());

    let _ = std::fs::remove_dir_all(temp_dir);
}

#[tokio::test]
async fn basic_scrape_opt_in_serves_metrics_with_two_factor_enabled() {
    let (state, temp_dir) = two_factor_auth_test_state_with("pending-metrics", true, |config| {
        config.metrics.allow_basic_scrape_with_2fa = true;
    })
    .await;
    let metrics = app(&state)
        .oneshot(request("/metrics", None))
        .await
        .expect("metrics response");
    assert_eq!(metrics.status(), StatusCode::OK);
    assert!(pending_cookie(&metrics).is_none());

    let page = app(&state)
        .oneshot(request("/api/overview", None))
        .await
        .expect("api response");
    assert_eq!(
        page.status(),
        StatusCode::FOUND,
        "the opt-in covers only /metrics"
    );

    let _ = std::fs::remove_dir_all(temp_dir);
}

#[tokio::test]
async fn a_live_pending_cookie_is_reused_instead_of_minting_another() {
    let (state, temp_dir) = two_factor_auth_test_state("pending-reuse", true).await;
    let first = app(&state)
        .oneshot(request("/api/overview", None))
        .await
        .expect("first response");
    assert_eq!(first.status(), StatusCode::FOUND);
    let cookie = pending_cookie(&first).expect("first request gets a pending cookie");

    for _ in 0..5 {
        let again = app(&state)
            .oneshot(request("/api/overview", Some(&cookie)))
            .await
            .expect("repeat response");
        assert_eq!(again.status(), StatusCode::FOUND);
        assert_eq!(
            again.headers().get(header::LOCATION).map(|v| v.as_bytes()),
            Some(b"/verify-2fa".as_slice())
        );
        assert!(pending_cookie(&again).is_none(), "no new pending session");
    }

    let token = cookie
        .split_once('=')
        .map(|(_, token)| token)
        .expect("cookie value");
    assert!(state.two_factor_sessions.pending_exists(token));

    let _ = std::fs::remove_dir_all(temp_dir);
}
