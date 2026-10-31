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
