//! Shared settings route fixtures keep concurrency regressions independent.

use super::*;

pub(super) const TEST_TOTP_SECRET: &str = "JBSWY3DPEHPK3PXP";
pub(super) static SETTINGS_TEST_ID: AtomicU64 = AtomicU64::new(1);

pub(super) struct SettingsHarness {
    pub(super) app: Router,
    pub(super) state: crate::AppState,
    pub(super) config_path: PathBuf,
    pub(super) temp_dir: PathBuf,
}

impl SettingsHarness {
    pub(super) async fn new(auth: ReadonlyAuthConfig) -> Result<Self> {
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

    pub(super) async fn cleanup(self) {
        self.state.history.shutdown().await;
        let _ = tokio::fs::remove_dir_all(&self.temp_dir).await;
    }
}

pub(super) fn settings_app(state: crate::AppState) -> Router {
    let protected_routes = Router::new()
        .route("/api/settings/password", post(change_readonly_password))
        .route(
            "/api/settings/alerts",
            post(crate::handlers::update_alert_settings),
        )
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

pub(super) async fn registered_node(
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

pub(super) fn readonly_auth(enable_2fa: bool, totp_secret: Option<&str>) -> ReadonlyAuthConfig {
    ReadonlyAuthConfig {
        username: "viewer".to_string(),
        password: "secret".to_string(),
        enable_2fa,
        totp_secret: totp_secret.map(str::to_string),
    }
}

pub(super) async fn write_server_config(
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

pub(super) async fn parse_current_config(
    config_path: &Path,
) -> Result<nodelite_proto::ServerConfig> {
    let content = tokio::fs::read_to_string(config_path).await?;
    Ok(parse_server_config(&content)?)
}

pub(super) fn basic_auth_header(password: &str) -> String {
    let encoded = base64::engine::general_purpose::STANDARD.encode(format!("viewer:{password}"));
    format!("Basic {encoded}")
}

pub(super) fn auth_cookie(token: &str) -> String {
    format!("{TWO_FACTOR_AUTH_COOKIE}={token}")
}

pub(super) fn json_request(
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

pub(super) fn json_delete_request(
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

pub(super) fn empty_post(uri: &str, authorization: &str, cookie: Option<&str>) -> Request<Body> {
    let mut builder = Request::builder()
        .method("POST")
        .uri(uri)
        .header(header::AUTHORIZATION, authorization);
    if let Some(cookie) = cookie {
        builder = builder.header(header::COOKIE, cookie);
    }
    builder.body(Body::empty()).expect("request should build")
}

pub(super) async fn response_json(response: Response<Body>) -> Result<Value> {
    let body = to_bytes(response.into_body(), usize::MAX).await?;
    Ok(serde_json::from_slice(&body)?)
}

pub(super) async fn assert_status(
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

pub(super) async fn current_totp_code_with_margin(secret: &str) -> String {
    while Utc::now().timestamp().rem_euclid(30) > 25 {
        sleep(Duration::from_millis(100)).await;
    }
    let secret = decode_totp_secret(secret).expect("test TOTP secret should decode");
    let step = Utc::now().timestamp().max(0) as u64 / 30;
    totp_custom::<Sha1>(30, 6, &secret, step.saturating_mul(30))
}
