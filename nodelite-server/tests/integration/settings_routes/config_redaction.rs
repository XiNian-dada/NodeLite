//! Invalid on-disk configuration must not leak values or partially apply settings changes.

use super::*;

#[tokio::test]
async fn config_errors_are_redacted_in_startup_and_settings_paths() -> Result<()> {
    let harness = SettingsHarness::new(readonly_auth(false, None)).await?;
    let original = tokio::fs::read_to_string(&harness.config_path).await?;
    let marker = "synthetic-private-config-value";
    let mut alert_request = serde_json::to_value(nodelite_proto::AlertingConfig::default())?;
    alert_request["current_password"] = json!("secret");
    alert_request["enabled"] = json!(true);
    for suffix in [
        format!("\n[geoip]\nprovider = '{marker}\n"),
        format!("\n[geoip]\nenabled = '{marker}'\n"),
    ] {
        let invalid = format!("{original}{suffix}");
        tokio::fs::write(&harness.config_path, &invalid).await?;
        let error = crate::startup::load_server_config(&harness.config_path)
            .await
            .expect_err("invalid configuration must prevent startup");
        for text in [
            error.to_string(),
            format!("{error:#}"),
            format!("{error:?}"),
        ] {
            assert!(
                !text.contains(marker),
                "startup diagnostic exposed an input value"
            );
        }
        assert!(format!("{error:#}").contains("line"));

        for (path, body, expected) in [
            (
                "/api/settings/password",
                json!({"current_password": "secret", "new_password": "ValidReplacement123!"}),
                StatusCode::INTERNAL_SERVER_ERROR,
            ),
            (
                "/api/settings/alerts",
                alert_request.clone(),
                if suffix.contains("enabled") {
                    StatusCode::BAD_REQUEST
                } else {
                    StatusCode::INTERNAL_SERVER_ERROR
                },
            ),
        ] {
            let response = harness
                .app
                .clone()
                .oneshot(json_request(path, &basic_auth_header("secret"), None, body))
                .await?;
            assert_eq!(response.status(), expected);
            let bytes = to_bytes(response.into_body(), 8192).await?;
            assert!(!String::from_utf8_lossy(&bytes).contains(marker));
            if path.ends_with("alerts") {
                assert!(String::from_utf8_lossy(&bytes).contains("line"));
                assert!(!harness.state.alerting.read().await.enabled);
            }
            assert_eq!(
                tokio::fs::read_to_string(&harness.config_path).await?,
                invalid
            );
            let auth = harness.state.readonly_auth.read().await;
            assert_eq!(
                auth.config.as_ref().map(|auth| auth.password.as_str()),
                Some("secret")
            );
        }
    }
    harness.cleanup().await;
    Ok(())
}
