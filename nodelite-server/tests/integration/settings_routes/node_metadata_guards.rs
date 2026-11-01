//! Node service metadata and location overrides follow the same auth and step-up rules as
//! every other settings write (#405).

use super::*;

const SERVICE_META_URI: &str = "/api/nodes/edge-guard-01/service-meta";
const LOCATION_URI: &str = "/api/nodes/edge-guard-01/location-override";

async fn issue_guard_node(harness: &SettingsHarness) -> Result<()> {
    issue_node(
        harness.state.registry.path(),
        IssueNodeRequest {
            node_id: "edge-guard-01".to_string(),
            node_label: None,
            tags: Vec::new(),
        },
    )
    .await?;
    harness.state.registry.reload().await?;
    Ok(())
}

fn throttling_service_meta() -> Value {
    json!({
        "service_expires_at": null,
        "service_unlimited": true,
        "renewal_price": null,
        "traffic_limit_bytes": 1024,
        "traffic_accounting": "bidirectional",
        "traffic_throttle_kbps": 8,
    })
}

fn location_override() -> Value {
    json!({
        "country": "HK",
        "city": "Hong Kong",
        "latitude": 22.3193,
        "longitude": 114.1694,
    })
}

async fn assert_node_unchanged(harness: &SettingsHarness) -> Result<()> {
    let node = registered_node(harness, "edge-guard-01").await?;
    assert_eq!(node.traffic_limit_bytes, None);
    assert_eq!(node.traffic_throttle_kbps, None);
    assert!(node.location_override().is_none());
    Ok(())
}

#[tokio::test]
async fn node_metadata_writes_are_rejected_when_auth_is_disabled() -> Result<()> {
    let harness = SettingsHarness::new(readonly_auth(false, None)).await?;
    issue_guard_node(&harness).await?;
    *harness.state.readonly_auth.write().await = crate::auth::ReadonlyRouteAuth::from_config(None);

    for (uri, body) in [
        (SERVICE_META_URI, throttling_service_meta()),
        (LOCATION_URI, location_override()),
    ] {
        let response = harness
            .app
            .clone()
            .oneshot(json_request(uri, "", None, body))
            .await?;
        assert_eq!(response.status(), StatusCode::CONFLICT, "{uri}");
    }
    assert_node_unchanged(&harness).await?;

    harness.cleanup().await;
    Ok(())
}

#[tokio::test]
async fn node_metadata_writes_require_step_up_confirmation() -> Result<()> {
    let harness = SettingsHarness::new(readonly_auth(false, None)).await?;
    issue_guard_node(&harness).await?;
    let token = harness
        .state
        .two_factor_sessions
        .create_basic_auth_session(None)?;
    let cookie = format!("{BASIC_AUTH_SESSION_COOKIE}={token}");

    for (uri, body) in [
        (SERVICE_META_URI, throttling_service_meta()),
        (LOCATION_URI, location_override()),
    ] {
        let response = harness
            .app
            .clone()
            .oneshot(json_request(
                uri,
                &basic_auth_header("secret"),
                Some(&cookie),
                body,
            ))
            .await?;
        assert_eq!(
            response.status(),
            StatusCode::PRECONDITION_REQUIRED,
            "{uri}"
        );
    }
    assert_node_unchanged(&harness).await?;

    harness
        .state
        .two_factor_sessions
        .confirm_sensitive_action(&token, false);
    for (uri, body) in [
        (SERVICE_META_URI, throttling_service_meta()),
        (LOCATION_URI, location_override()),
    ] {
        let response = harness
            .app
            .clone()
            .oneshot(json_request(
                uri,
                &basic_auth_header("secret"),
                Some(&cookie),
                body,
            ))
            .await?;
        assert_status(response.status(), StatusCode::OK, response).await?;
    }
    let node = registered_node(&harness, "edge-guard-01").await?;
    assert_eq!(node.traffic_limit_bytes, Some(1024));
    assert_eq!(node.traffic_throttle_kbps, Some(8));
    assert!(node.location_override().is_some());

    harness.cleanup().await;
    Ok(())
}

fn with_confirmation(mut body: Value, field: &str, value: &str) -> Value {
    body[field] = json!(value);
    body
}

#[tokio::test]
async fn node_metadata_writes_accept_inline_password_confirmation() -> Result<()> {
    let harness = SettingsHarness::new(readonly_auth(false, None)).await?;
    issue_guard_node(&harness).await?;

    for (uri, body) in [
        (SERVICE_META_URI, throttling_service_meta()),
        (LOCATION_URI, location_override()),
    ] {
        let wrong = with_confirmation(body, "current_password", "not-the-password");
        let response = harness
            .app
            .clone()
            .oneshot(json_request(uri, &basic_auth_header("secret"), None, wrong))
            .await?;
        assert_eq!(response.status(), StatusCode::FORBIDDEN, "{uri}");
    }
    assert_node_unchanged(&harness).await?;

    for (uri, body) in [
        (SERVICE_META_URI, throttling_service_meta()),
        (LOCATION_URI, location_override()),
    ] {
        let confirmed = with_confirmation(body, "current_password", "secret");
        let response = harness
            .app
            .clone()
            .oneshot(json_request(
                uri,
                &basic_auth_header("secret"),
                None,
                confirmed,
            ))
            .await?;
        assert_status(response.status(), StatusCode::OK, response).await?;
    }
    let node = registered_node(&harness, "edge-guard-01").await?;
    assert_eq!(node.traffic_throttle_kbps, Some(8));
    assert!(node.location_override().is_some());

    harness.cleanup().await;
    Ok(())
}

#[tokio::test]
async fn node_service_metadata_accepts_inline_totp_confirmation() -> Result<()> {
    let harness = SettingsHarness::new(readonly_auth(true, Some(TEST_TOTP_SECRET))).await?;
    issue_guard_node(&harness).await?;
    let token = harness.state.two_factor_sessions.create_authenticated()?;
    let code = current_totp_code_with_margin(TEST_TOTP_SECRET).await;

    let body = with_confirmation(throttling_service_meta(), "code", &code);
    let response = harness
        .app
        .clone()
        .oneshot(json_request(
            SERVICE_META_URI,
            "",
            Some(&auth_cookie(&token)),
            body,
        ))
        .await?;
    assert_status(response.status(), StatusCode::OK, response).await?;
    let node = registered_node(&harness, "edge-guard-01").await?;
    assert_eq!(node.traffic_throttle_kbps, Some(8));

    harness.cleanup().await;
    Ok(())
}

async fn post_service_meta(harness: &SettingsHarness, body: Value) -> Result<()> {
    let body = with_confirmation(body, "current_password", "secret");
    let response = harness
        .app
        .clone()
        .oneshot(json_request(
            SERVICE_META_URI,
            &basic_auth_header("secret"),
            None,
            body,
        ))
        .await?;
    assert_status(response.status(), StatusCode::OK, response).await
}

#[tokio::test]
async fn service_metadata_omitted_fields_keep_stored_values() -> Result<()> {
    let harness = SettingsHarness::new(readonly_auth(false, None)).await?;
    issue_guard_node(&harness).await?;
    post_service_meta(
        &harness,
        json!({
            "service_unlimited": false,
            "service_expires_at": "2027-01-01T00:00:00Z",
            "renewal_price": "$5/mo",
            "traffic_limit_bytes": 4096,
            "traffic_accounting": "outbound",
            "traffic_throttle_kbps": 16,
        }),
    )
    .await?;

    post_service_meta(
        &harness,
        json!({ "service_expires_at": "2027-06-01T00:00:00Z" }),
    )
    .await?;
    let node = registered_node(&harness, "edge-guard-01").await?;
    assert_eq!(
        node.service_expires_at.map(|at| at.to_rfc3339()),
        Some("2027-06-01T00:00:00+00:00".to_string())
    );
    assert_eq!(node.renewal_price.as_deref(), Some("$5/mo"));
    assert_eq!(node.traffic_limit_bytes, Some(4096));
    assert_eq!(
        node.traffic_accounting,
        crate::registry::TrafficAccounting::Outbound
    );
    assert_eq!(node.traffic_throttle_kbps, Some(16));

    post_service_meta(&harness, json!({ "traffic_throttle_kbps": null })).await?;
    let node = registered_node(&harness, "edge-guard-01").await?;
    assert_eq!(node.traffic_throttle_kbps, None, "explicit null clears");
    assert_eq!(node.traffic_limit_bytes, Some(4096));

    harness.cleanup().await;
    Ok(())
}

#[tokio::test]
async fn stored_zero_limit_does_not_block_edits_to_other_fields() -> Result<()> {
    let harness = SettingsHarness::new(readonly_auth(false, None)).await?;
    issue_guard_node(&harness).await?;
    let path = harness.state.registry.path().to_path_buf();
    let mut file: Value = serde_json::from_str(&tokio::fs::read_to_string(&path).await?)?;
    file["nodes"][0]["traffic_limit_bytes"] = json!(0);
    tokio::fs::write(&path, serde_json::to_string_pretty(&file)?).await?;
    harness.state.registry.reload().await?;

    post_service_meta(&harness, json!({ "renewal_price": "$7/mo" })).await?;
    let node = registered_node(&harness, "edge-guard-01").await?;
    assert_eq!(node.renewal_price.as_deref(), Some("$7/mo"));
    assert_eq!(node.traffic_limit_bytes, Some(0));

    let rejected = harness
        .app
        .clone()
        .oneshot(json_request(
            SERVICE_META_URI,
            &basic_auth_header("secret"),
            None,
            with_confirmation(
                json!({ "traffic_limit_bytes": 0 }),
                "current_password",
                "secret",
            ),
        ))
        .await?;
    assert_eq!(rejected.status(), StatusCode::BAD_REQUEST);

    harness.cleanup().await;
    Ok(())
}

async fn first_blocked_attempt(
    harness: &SettingsHarness,
    authorization: &str,
    cookie: Option<&str>,
    field: &str,
) -> Result<Option<usize>> {
    for attempt in 0..50 {
        let body = with_confirmation(throttling_service_meta(), field, "000000-wrong");
        let response = harness
            .app
            .clone()
            .oneshot(json_request(SERVICE_META_URI, authorization, cookie, body))
            .await?;
        match response.status() {
            StatusCode::FORBIDDEN => continue,
            StatusCode::TOO_MANY_REQUESTS => return Ok(Some(attempt)),
            other => anyhow::bail!("unexpected status {other} on attempt {attempt}"),
        }
    }
    Ok(None)
}

#[tokio::test]
async fn repeated_wrong_inline_passwords_hit_the_sensitive_failure_budget() -> Result<()> {
    let harness = SettingsHarness::new(readonly_auth(false, None)).await?;
    issue_guard_node(&harness).await?;
    let blocked_at = first_blocked_attempt(
        &harness,
        &basic_auth_header("secret"),
        None,
        "current_password",
    )
    .await?;
    assert!(
        blocked_at.is_some(),
        "inline password guesses must be limited"
    );

    let correct = with_confirmation(throttling_service_meta(), "current_password", "secret");
    let response = harness
        .app
        .clone()
        .oneshot(json_request(
            SERVICE_META_URI,
            &basic_auth_header("secret"),
            None,
            correct,
        ))
        .await?;
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_node_unchanged(&harness).await?;

    harness.cleanup().await;
    Ok(())
}

#[tokio::test]
async fn signed_in_two_factor_session_cannot_keep_guessing_inline_codes() -> Result<()> {
    let harness = SettingsHarness::new(readonly_auth(true, Some(TEST_TOTP_SECRET))).await?;
    issue_guard_node(&harness).await?;
    let token = harness.state.two_factor_sessions.create_authenticated()?;
    harness
        .state
        .two_factor_sessions
        .expire_sensitive_confirmation_for_test(&token);
    let blocked_at =
        first_blocked_attempt(&harness, "", Some(&auth_cookie(&token)), "code").await?;
    assert!(blocked_at.is_some(), "inline TOTP guesses must be limited");
    assert_node_unchanged(&harness).await?;

    harness.cleanup().await;
    Ok(())
}
