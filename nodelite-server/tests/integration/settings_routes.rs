use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Request, Response, StatusCode, header};
use axum::middleware::{from_fn, from_fn_with_state};
use axum::routing::{delete, post};
use base64::Engine;
use chrono::Utc;
use serde_json::{Value, json};
use tokio::time::sleep;
use totp_lite::{Sha1, totp_custom};
use tower::ServiceExt;

use super::*;
use crate::auth::{TWO_FACTOR_AUTH_COOKIE, decode_totp_secret};
use crate::handlers::{
    change_readonly_password, delete_agent, disable_two_factor, enable_two_factor,
    refresh_node_token, require_readonly_auth, start_server_update, start_two_factor_setup,
    update_node_location_override,
};
use crate::registry::{IssueNodeRequest, issue_node};
use crate::set_protected_response_headers;
use crate::snapshot::{load_snapshot, persist_snapshot};
use crate::state::{SessionCommand, SessionControlHandle, SessionRefreshReply};
use crate::test_support::{fake_snapshot, synthetic_identity, test_server_config};
use nodelite_proto::{GeoIpLocation, ReadonlyAuthConfig, parse_server_config};

const TEST_TOTP_SECRET: &str = "JBSWY3DPEHPK3PXP";
static SETTINGS_TEST_ID: AtomicU64 = AtomicU64::new(1);

struct SettingsHarness {
    app: Router,
    state: crate::AppState,
    config_path: PathBuf,
    temp_dir: PathBuf,
}

impl SettingsHarness {
    async fn new(auth: ReadonlyAuthConfig) -> Result<Self> {
        let unique = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let sequence = SETTINGS_TEST_ID.fetch_add(1, Ordering::Relaxed);
        let temp_dir = std::env::temp_dir().join(format!(
            "nodelite-settings-routes-{}-{unique}-{sequence}",
            std::process::id(),
        ));
        tokio::fs::create_dir_all(&temp_dir).await?;
        let registry_path = temp_dir.join("server.json");
        let history_path = temp_dir.join("history.sqlite3");
        let snapshot_path = temp_dir.join("snapshot.json");
        let config_path = temp_dir.join("server.toml");
        write_server_config(
            &config_path,
            &registry_path,
            &history_path,
            &snapshot_path,
            &auth,
        )
        .await?;

        let mut config = test_server_config(
            SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 8080)),
            "https://monitor.example.com".to_string(),
            registry_path,
            history_path,
            snapshot_path,
        );
        config.readonly_auth = Some(auth);
        let state =
            crate::AppState::test_fixture(config.into(), Arc::new(config_path.clone())).await?;
        let app = settings_app(state.clone());
        Ok(Self {
            app,
            state,
            config_path,
            temp_dir,
        })
    }

    async fn cleanup(self) {
        self.state.history.shutdown().await;
        let _ = tokio::fs::remove_dir_all(&self.temp_dir).await;
    }
}

#[tokio::test]
async fn settings_password_change_covers_failure_and_persistence_paths() -> Result<()> {
    let harness = SettingsHarness::new(readonly_auth(false, None)).await?;

    let rejected = harness
        .app
        .clone()
        .oneshot(json_request(
            "/api/settings/password",
            &basic_auth_header("secret"),
            None,
            json!({
                "current_password": "wrong",
                "new_password": "VeryStrong123!",
            }),
        ))
        .await?;
    assert_eq!(rejected.status(), StatusCode::UNAUTHORIZED);

    let changed = harness
        .app
        .clone()
        .oneshot(json_request(
            "/api/settings/password",
            &basic_auth_header("secret"),
            None,
            json!({
                "current_password": "secret",
                "new_password": "VeryStrong123!",
            }),
        ))
        .await?;
    assert_eq!(changed.status(), StatusCode::OK);

    let persisted = parse_current_config(&harness.config_path).await?;
    assert_eq!(
        persisted
            .readonly_auth
            .as_ref()
            .map(|auth| auth.password.as_str()),
        Some("VeryStrong123!"),
    );
    let runtime_auth = harness.state.readonly_auth.read().await;
    assert_eq!(
        runtime_auth.expected_authorization.as_deref(),
        Some(basic_auth_header("VeryStrong123!").as_str()),
    );
    drop(runtime_auth);

    let old_header_rejected = harness
        .app
        .clone()
        .oneshot(json_request(
            "/api/settings/password",
            &basic_auth_header("secret"),
            None,
            json!({
                "current_password": "VeryStrong123!",
                "new_password": "AnotherStrong123!",
            }),
        ))
        .await?;
    assert_eq!(old_header_rejected.status(), StatusCode::UNAUTHORIZED);

    let weak_password_rejected = harness
        .app
        .clone()
        .oneshot(json_request(
            "/api/settings/password",
            &basic_auth_header("VeryStrong123!"),
            None,
            json!({
                "current_password": "VeryStrong123!",
                "new_password": "short",
            }),
        ))
        .await?;
    assert_eq!(weak_password_rejected.status(), StatusCode::BAD_REQUEST);

    harness.cleanup().await;
    Ok(())
}

#[tokio::test]
async fn settings_two_factor_enable_rejects_replayed_totp() -> Result<()> {
    let harness = SettingsHarness::new(readonly_auth(false, None)).await?;

    let setup = harness
        .app
        .clone()
        .oneshot(empty_post(
            "/api/settings/2fa/start",
            &basic_auth_header("secret"),
            None,
        ))
        .await?;
    assert_eq!(setup.status(), StatusCode::OK);
    let setup_body = response_json(setup).await?;
    let secret = setup_body["secret"]
        .as_str()
        .expect("setup response should include secret")
        .to_string();
    assert!(
        setup_body["otpauth_uri"]
            .as_str()
            .is_some_and(|uri| uri.contains(&secret))
    );

    let code = current_totp_code_with_margin(&secret).await;
    let enabled = harness
        .app
        .clone()
        .oneshot(json_request(
            "/api/settings/2fa/enable",
            &basic_auth_header("secret"),
            None,
            json!({
                "current_password": "secret",
                "secret": secret,
                "code": code,
            }),
        ))
        .await?;
    assert_status(enabled.status(), StatusCode::OK, enabled).await?;
    let persisted = parse_current_config(&harness.config_path).await?;
    let auth = persisted
        .readonly_auth
        .expect("auth should remain configured");
    assert!(auth.enable_2fa);
    assert!(auth.totp_secret.is_some());

    let auth_token = harness.state.two_factor_sessions.create_authenticated()?;
    let replayed = harness
        .app
        .clone()
        .oneshot(json_request(
            "/api/settings/2fa/disable",
            &basic_auth_header("secret"),
            Some(&auth_cookie(&auth_token)),
            json!({
                "current_password": "secret",
                "code": code,
            }),
        ))
        .await?;
    assert_eq!(replayed.status(), StatusCode::UNAUTHORIZED);
    let replay_body = response_json(replayed).await?;
    assert_eq!(replay_body["message"], "verification code already used");

    harness.cleanup().await;
    Ok(())
}

#[tokio::test]
async fn settings_two_factor_disable_covers_password_failure_and_persistence() -> Result<()> {
    let harness = SettingsHarness::new(readonly_auth(true, Some(TEST_TOTP_SECRET))).await?;
    let auth_token = harness.state.two_factor_sessions.create_authenticated()?;
    let code = current_totp_code_with_margin(TEST_TOTP_SECRET).await;

    let wrong_password = harness
        .app
        .clone()
        .oneshot(json_request(
            "/api/settings/2fa/disable",
            &basic_auth_header("secret"),
            Some(&auth_cookie(&auth_token)),
            json!({
                "current_password": "wrong",
                "code": code,
            }),
        ))
        .await?;
    assert_eq!(wrong_password.status(), StatusCode::UNAUTHORIZED);

    let disabled = harness
        .app
        .clone()
        .oneshot(json_request(
            "/api/settings/2fa/disable",
            &basic_auth_header("secret"),
            Some(&auth_cookie(&auth_token)),
            json!({
                "current_password": "secret",
                "code": code,
            }),
        ))
        .await?;
    assert_status(disabled.status(), StatusCode::OK, disabled).await?;
    let persisted = parse_current_config(&harness.config_path).await?;
    let auth = persisted
        .readonly_auth
        .expect("auth should remain configured");
    assert!(!auth.enable_2fa);
    assert!(auth.totp_secret.is_none());

    harness.cleanup().await;
    Ok(())
}

#[tokio::test]
async fn settings_node_token_refresh_covers_success_offline_and_timeout_paths() -> Result<()> {
    let harness = SettingsHarness::new(readonly_auth(false, None)).await?;

    let online_session = harness
        .state
        .shared
        .register_node(
            synthetic_identity(
                "online-refresh-01",
                "Online Refresh 01",
                "test",
                None,
                "itest",
            ),
            Some("127.0.0.1".to_string()),
            None,
            None,
        )
        .await;
    harness
        .state
        .shared
        .update_snapshot("online-refresh-01", online_session, fake_snapshot(1))
        .await;
    let (control, mut control_rx) = SessionControlHandle::channel();
    assert!(
        harness
            .state
            .shared
            .attach_session_control("online-refresh-01", online_session, control)
            .await
    );
    let refresh_task = tokio::spawn(async move {
        let Some(SessionCommand::RefreshToken {
            response,
            refresh_permit: _refresh_permit,
        }) = control_rx.recv().await
        else {
            return;
        };
        let _ = response.send(Ok(SessionRefreshReply {
            token_expires_at: Utc::now() + chrono::Duration::days(30),
        }));
    });

    let refreshed = harness
        .app
        .clone()
        .oneshot(json_request(
            "/api/nodes/online-refresh-01/refresh-token",
            &basic_auth_header("secret"),
            None,
            json!({ "current_password": "secret" }),
        ))
        .await?;
    assert_eq!(refreshed.status(), StatusCode::OK);
    refresh_task.await?;

    let offline_session = harness
        .state
        .shared
        .register_node(
            synthetic_identity(
                "offline-refresh-01",
                "Offline Refresh 01",
                "test",
                None,
                "itest",
            ),
            None,
            None,
            None,
        )
        .await;
    harness
        .state
        .shared
        .mark_disconnected("offline-refresh-01", offline_session)
        .await;
    let offline = harness
        .app
        .clone()
        .oneshot(json_request(
            "/api/nodes/offline-refresh-01/refresh-token",
            &basic_auth_header("secret"),
            None,
            json!({ "current_password": "secret" }),
        ))
        .await?;
    assert_eq!(offline.status(), StatusCode::CONFLICT);

    let timeout_session = harness
        .state
        .shared
        .register_node(
            synthetic_identity(
                "timeout-refresh-01",
                "Timeout Refresh 01",
                "test",
                None,
                "itest",
            ),
            None,
            None,
            None,
        )
        .await;
    let (timeout_control, _timeout_rx) = SessionControlHandle::channel();
    assert!(
        harness
            .state
            .shared
            .attach_session_control("timeout-refresh-01", timeout_session, timeout_control)
            .await
    );
    let timed_out = harness
        .app
        .clone()
        .oneshot(json_request(
            "/api/nodes/timeout-refresh-01/refresh-token",
            &basic_auth_header("secret"),
            None,
            json!({ "current_password": "secret" }),
        ))
        .await?;
    assert_eq!(timed_out.status(), StatusCode::GATEWAY_TIMEOUT);

    harness.cleanup().await;
    Ok(())
}

#[tokio::test]
async fn settings_node_location_override_persists_and_updates_runtime_view() -> Result<()> {
    let harness = SettingsHarness::new(readonly_auth(false, None)).await?;
    issue_node(
        harness.state.registry.path(),
        IssueNodeRequest {
            node_id: "edge-hkg-01".to_string(),
            node_label: Some("Edge HKG 01".to_string()),
            tags: Vec::new(),
        },
    )
    .await?;
    harness.state.registry.reload().await?;

    let session_id = harness
        .state
        .shared
        .register_node(
            synthetic_identity("edge-hkg-01", "Edge HKG 01", "test", None, "itest"),
            Some("203.0.113.7".to_string()),
            Some(GeoIpLocation {
                country: "CN".to_string(),
                city: Some("Shenyang".to_string()),
                latitude: Some(41.8057),
                longitude: Some(123.4315),
            }),
            None,
        )
        .await;
    harness
        .state
        .shared
        .update_snapshot("edge-hkg-01", session_id, fake_snapshot(1))
        .await;

    let saved = harness
        .app
        .clone()
        .oneshot(json_request(
            "/api/nodes/edge-hkg-01/location-override",
            &basic_auth_header("secret"),
            None,
            json!({
                "country": "香港",
                "city": "香港",
                "latitude": 22.3193,
                "longitude": 114.1694,
            }),
        ))
        .await?;
    assert_status(saved.status(), StatusCode::OK, saved).await?;

    let registered = registered_node(&harness, "edge-hkg-01").await?;
    let registry_override = registered
        .location_override()
        .expect("registry should retain manual location");
    assert_eq!(registry_override.country, "香港");
    assert_eq!(registry_override.city.as_deref(), Some("香港"));
    assert_eq!(registry_override.latitude, Some(22.3193));
    assert_eq!(registry_override.longitude, Some(114.1694));

    let status = harness
        .state
        .shared
        .get_status("edge-hkg-01")
        .await
        .expect("runtime node should exist");
    assert_eq!(status.geoip_country.as_deref(), Some("CN"));
    assert_eq!(status.geoip_city.as_deref(), Some("Shenyang"));
    assert_eq!(status.location_override_country.as_deref(), Some("香港"));
    assert_eq!(status.location_override_city.as_deref(), Some("香港"));
    assert_eq!(status.location_override_latitude, Some(22.3193));
    assert_eq!(status.location_override_longitude, Some(114.1694));

    let cleared = harness
        .app
        .clone()
        .oneshot(json_request(
            "/api/nodes/edge-hkg-01/location-override",
            &basic_auth_header("secret"),
            None,
            json!({
                "country": null,
                "city": null,
                "latitude": null,
                "longitude": null,
            }),
        ))
        .await?;
    assert_status(cleared.status(), StatusCode::OK, cleared).await?;

    let registered = registered_node(&harness, "edge-hkg-01").await?;
    assert!(registered.location_override().is_none());
    let status = harness
        .state
        .shared
        .get_status("edge-hkg-01")
        .await
        .expect("runtime node should remain visible");
    assert_eq!(status.geoip_country.as_deref(), Some("CN"));
    assert!(status.location_override_country.is_none());
    assert!(status.location_override_city.is_none());
    assert!(status.location_override_latitude.is_none());
    assert!(status.location_override_longitude.is_none());

    harness.cleanup().await;
    Ok(())
}

#[tokio::test]
async fn settings_agent_delete_revokes_enrollment_and_removes_runtime_node() -> Result<()> {
    let harness = SettingsHarness::new(readonly_auth(false, None)).await?;
    let issued = issue_node(
        harness.state.registry.path(),
        IssueNodeRequest {
            node_id: "edge-sin-01".to_string(),
            node_label: Some("Edge SIN 01".to_string()),
            tags: Vec::new(),
        },
    )
    .await?;
    harness.state.registry.reload().await?;
    harness
        .state
        .shared
        .register_node(
            synthetic_identity("edge-sin-01", "Edge SIN 01", "test", None, "itest"),
            Some("203.0.113.8".to_string()),
            None,
            None,
        )
        .await;
    assert!(
        harness
            .state
            .shared
            .get_status("edge-sin-01")
            .await
            .is_some()
    );
    let snapshot_path = harness.state.shared.config().snapshot_path.clone();
    persist_snapshot(
        snapshot_path.as_path(),
        &harness.state.shared.list_statuses().await,
    )
    .await?;

    let rejected = harness
        .app
        .clone()
        .oneshot(json_delete_request(
            "/api/settings/agents/edge-sin-01",
            &basic_auth_header("secret"),
            None,
            json!({ "current_password": "wrong" }),
        ))
        .await?;
    assert_eq!(rejected.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(harness.state.registry.count().await, 1);

    let deleted = harness
        .app
        .clone()
        .oneshot(json_delete_request(
            "/api/settings/agents/edge-sin-01",
            &basic_auth_header("secret"),
            None,
            json!({ "current_password": "secret" }),
        ))
        .await?;
    assert_status(deleted.status(), StatusCode::OK, deleted).await?;

    assert!(
        harness
            .state
            .registry
            .list_registered_nodes()
            .await
            .is_empty()
    );
    assert!(
        harness
            .state
            .shared
            .get_status("edge-sin-01")
            .await
            .is_none()
    );
    assert!(load_snapshot(snapshot_path.as_path()).await?.is_empty());
    assert!(
        harness
            .state
            .registry
            .consume_install_token(&issued.install_token)
            .await?
            .is_none()
    );

    let missing = harness
        .app
        .clone()
        .oneshot(json_delete_request(
            "/api/settings/agents/edge-sin-01",
            &basic_auth_header("secret"),
            None,
            json!({ "current_password": "secret" }),
        ))
        .await?;
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);

    harness.cleanup().await;
    Ok(())
}

#[tokio::test]
async fn settings_server_update_requires_sensitive_confirmation() -> Result<()> {
    let harness = SettingsHarness::new(readonly_auth(false, None)).await?;

    let missing_password = harness
        .app
        .clone()
        .oneshot(json_request(
            "/api/settings/update/server",
            &basic_auth_header("secret"),
            None,
            json!({}),
        ))
        .await?;
    assert_eq!(missing_password.status(), StatusCode::UNAUTHORIZED);

    let wrong_password = harness
        .app
        .clone()
        .oneshot(json_request(
            "/api/settings/update/server",
            &basic_auth_header("secret"),
            None,
            json!({ "current_password": "wrong" }),
        ))
        .await?;
    assert_eq!(wrong_password.status(), StatusCode::UNAUTHORIZED);

    harness.cleanup().await;
    Ok(())
}

fn settings_app(state: crate::AppState) -> Router {
    let protected_routes = Router::new()
        .route("/api/settings/password", post(change_readonly_password))
        .route("/api/settings/agents/{node_id}", delete(delete_agent))
        .route("/api/settings/update/server", post(start_server_update))
        .route("/api/settings/2fa/start", post(start_two_factor_setup))
        .route("/api/settings/2fa/enable", post(enable_two_factor))
        .route("/api/settings/2fa/disable", post(disable_two_factor))
        .route(
            "/api/nodes/{node_id}/refresh-token",
            post(refresh_node_token),
        )
        .route(
            "/api/nodes/{node_id}/location-override",
            post(update_node_location_override),
        )
        .route_layer(from_fn(set_protected_response_headers))
        .route_layer(from_fn_with_state(state.clone(), require_readonly_auth));
    Router::new().merge(protected_routes).with_state(state)
}

async fn registered_node(
    harness: &SettingsHarness,
    node_id: &str,
) -> Result<crate::registry::RegisteredNode> {
    harness
        .state
        .registry
        .list_registered_nodes()
        .await
        .into_iter()
        .find(|node| node.node_id == node_id)
        .ok_or_else(|| anyhow::anyhow!("registered node {node_id} not found"))
}

fn readonly_auth(enable_2fa: bool, totp_secret: Option<&str>) -> ReadonlyAuthConfig {
    ReadonlyAuthConfig {
        username: "viewer".to_string(),
        password: "secret".to_string(),
        enable_2fa,
        totp_secret: totp_secret.map(str::to_string),
    }
}

async fn write_server_config(
    config_path: &Path,
    registry_path: &Path,
    history_path: &Path,
    snapshot_path: &Path,
    auth: &ReadonlyAuthConfig,
) -> Result<()> {
    let mut content = format!(
        r#"[server]
listen = "127.0.0.1:8080"
public_base_url = "https://monitor.example.com"
node_registry_path = "{}"
history_db_path = "{}"
snapshot_path = "{}"

[auth]
username = "{}"
password = "{}"
enable_2fa = {}
"#,
        registry_path.display(),
        history_path.display(),
        snapshot_path.display(),
        auth.username,
        auth.password,
        auth.enable_2fa,
    );
    if let Some(secret) = auth.totp_secret.as_deref() {
        content.push_str(&format!("totp_secret = \"{secret}\"\n"));
    }
    tokio::fs::write(config_path, content).await?;
    Ok(())
}

async fn parse_current_config(config_path: &Path) -> Result<nodelite_proto::ServerConfig> {
    let content = tokio::fs::read_to_string(config_path).await?;
    Ok(parse_server_config(&content)?)
}

fn basic_auth_header(password: &str) -> String {
    let encoded = base64::engine::general_purpose::STANDARD.encode(format!("viewer:{password}"));
    format!("Basic {encoded}")
}

fn auth_cookie(token: &str) -> String {
    format!("{TWO_FACTOR_AUTH_COOKIE}={token}")
}

fn json_request(
    uri: &str,
    authorization: &str,
    cookie: Option<&str>,
    body: Value,
) -> Request<Body> {
    let mut builder = Request::builder()
        .method("POST")
        .uri(uri)
        .header(header::AUTHORIZATION, authorization)
        .header(header::CONTENT_TYPE, "application/json");
    if let Some(cookie) = cookie {
        builder = builder.header(header::COOKIE, cookie);
    }
    builder
        .body(Body::from(body.to_string()))
        .expect("request should build")
}

fn json_delete_request(
    uri: &str,
    authorization: &str,
    cookie: Option<&str>,
    body: Value,
) -> Request<Body> {
    let mut builder = Request::builder()
        .method("DELETE")
        .uri(uri)
        .header(header::AUTHORIZATION, authorization)
        .header(header::CONTENT_TYPE, "application/json");
    if let Some(cookie) = cookie {
        builder = builder.header(header::COOKIE, cookie);
    }
    builder
        .body(Body::from(body.to_string()))
        .expect("request should build")
}

fn empty_post(uri: &str, authorization: &str, cookie: Option<&str>) -> Request<Body> {
    let mut builder = Request::builder()
        .method("POST")
        .uri(uri)
        .header(header::AUTHORIZATION, authorization);
    if let Some(cookie) = cookie {
        builder = builder.header(header::COOKIE, cookie);
    }
    builder.body(Body::empty()).expect("request should build")
}

async fn response_json(response: Response<Body>) -> Result<Value> {
    let body = to_bytes(response.into_body(), usize::MAX).await?;
    Ok(serde_json::from_slice(&body)?)
}

async fn assert_status(
    actual: StatusCode,
    expected: StatusCode,
    response: Response<Body>,
) -> Result<()> {
    if actual == expected {
        return Ok(());
    }
    let body = to_bytes(response.into_body(), usize::MAX).await?;
    anyhow::bail!(
        "expected status {expected}, got {actual}; body: {}",
        String::from_utf8_lossy(&body)
    );
}

async fn current_totp_code_with_margin(secret: &str) -> String {
    while Utc::now().timestamp().rem_euclid(30) > 25 {
        sleep(Duration::from_millis(100)).await;
    }
    let secret = decode_totp_secret(secret).expect("test TOTP secret should decode");
    let step = Utc::now().timestamp().max(0) as u64 / 30;
    totp_custom::<Sha1>(30, 6, &secret, step.saturating_mul(30))
}
