//! New alert settings writes must stay within the bounds that loading only clamps.

use super::*;

#[tokio::test]
async fn alert_settings_reject_values_above_load_time_clamps() -> Result<()> {
    let harness = SettingsHarness::new(readonly_auth(false, None)).await?;
    let original = tokio::fs::read_to_string(&harness.config_path).await?;
    let mut request = serde_json::to_value(nodelite_proto::AlertingConfig::default())?;
    request["current_password"] = json!("secret");
    request["inspection"]["lookback_hours"] = json!(337);

    let response = harness
        .app
        .clone()
        .oneshot(json_request(
            "/api/settings/alerts",
            &basic_auth_header("secret"),
            None,
            request,
        ))
        .await?;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let bytes = to_bytes(response.into_body(), 8192).await?;
    assert!(String::from_utf8_lossy(&bytes).contains("alerts.inspection.lookback_hours"));
    assert_eq!(
        tokio::fs::read_to_string(&harness.config_path).await?,
        original
    );
    assert_ne!(
        harness
            .state
            .alerting
            .read()
            .await
            .inspection
            .lookback_hours,
        337
    );
    harness.cleanup().await;
    Ok(())
}
