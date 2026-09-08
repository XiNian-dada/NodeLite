//! Regression coverage for the shared disk/runtime settings transaction.

use super::*;
use tokio::time::timeout;

fn alert_request() -> Value {
    let mut body = serde_json::to_value(nodelite_proto::AlertingConfig::default())
        .expect("serialize default alerts");
    body["current_password"] = json!("secret");
    body["inspection"]["cpu_warn_percent"] = json!(73);
    body
}

#[tokio::test]
async fn concurrent_settings_preserve_every_successful_change_on_disk_and_in_runtime() -> Result<()>
{
    let harness = SettingsHarness::new(readonly_auth(false, None)).await?;
    let guard = harness.state.settings_write_lock.lock().await;
    let code = current_totp_code_with_margin(TEST_TOTP_SECRET).await;
    let requests = [
        ("/api/settings/alerts", alert_request()),
        (
            "/api/settings/2fa/enable",
            json!({
                "current_password": "secret", "secret": TEST_TOTP_SECRET, "code": code,
            }),
        ),
        (
            "/api/settings/password",
            json!({
                "current_password": "secret", "new_password": "VeryStrong123!",
            }),
        ),
    ];
    let mut pending = Vec::new();
    for (path, body) in requests {
        let mut response = Box::pin(harness.app.clone().oneshot(json_request(
            path,
            &basic_auth_header("secret"),
            None,
            body,
        )));
        // Poll each HTTP request through authentication while commits are blocked.
        assert!(
            timeout(Duration::from_millis(20), &mut response)
                .await
                .is_err()
        );
        pending.push(response);
    }
    drop(guard);
    for response in pending {
        let response = timeout(Duration::from_secs(5), response).await??;
        assert_status(response.status(), StatusCode::OK, response).await?;
    }
    assert_concurrent_changes(&harness).await?;
    harness.cleanup().await;
    Ok(())
}

async fn assert_concurrent_changes(harness: &SettingsHarness) -> Result<()> {
    let persisted = parse_current_config(&harness.config_path).await?;
    let auth = persisted.readonly_auth.expect("auth retained");
    assert_eq!(auth.password, "VeryStrong123!");
    assert!(auth.enable_2fa);
    assert_eq!(auth.totp_secret.as_deref(), Some(TEST_TOTP_SECRET));
    assert_eq!(persisted.alerting.inspection.cpu_warn_percent, 73);
    let runtime = harness
        .state
        .readonly_auth
        .read()
        .await
        .config
        .clone()
        .expect("runtime auth retained");
    assert_eq!(runtime.password, auth.password);
    assert_eq!(runtime.enable_2fa, auth.enable_2fa);
    assert_eq!(runtime.totp_secret, auth.totp_secret);
    assert_eq!(
        harness
            .state
            .alerting
            .read()
            .await
            .inspection
            .cpu_warn_percent,
        73
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&harness.config_path)?
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
    assert!(!harness.config_path.with_extension("toml.tmp").exists());
    Ok(())
}

#[tokio::test]
async fn disable_two_factor_uses_the_same_commit_boundary() -> Result<()> {
    let harness = SettingsHarness::new(readonly_auth(true, Some(TEST_TOTP_SECRET))).await?;
    let token = harness.state.two_factor_sessions.create_authenticated()?;
    let guard = harness.state.settings_write_lock.lock().await;
    let mut response = Box::pin(harness.app.clone().oneshot(json_request(
        "/api/settings/2fa/disable", &basic_auth_header("secret"), Some(&auth_cookie(&token)),
        json!({"current_password": "secret", "code": current_totp_code_with_margin(TEST_TOTP_SECRET).await}),
    )));
    assert!(
        timeout(Duration::from_millis(20), &mut response)
            .await
            .is_err()
    );
    assert!(
        parse_current_config(&harness.config_path)
            .await?
            .readonly_auth
            .expect("auth")
            .enable_2fa
    );
    drop(guard);
    assert_eq!(response.await?.status(), StatusCode::OK);
    assert!(
        !parse_current_config(&harness.config_path)
            .await?
            .readonly_auth
            .expect("auth")
            .enable_2fa
    );
    assert!(!harness.state.readonly_auth.read().await.enable_2fa);
    harness.cleanup().await;
    Ok(())
}

#[tokio::test]
async fn failed_settings_write_does_not_update_runtime() -> Result<()> {
    let harness = SettingsHarness::new(readonly_auth(false, None)).await?;
    let original = tokio::fs::read(&harness.config_path).await?;
    tokio::fs::remove_file(&harness.config_path).await?;
    tokio::fs::create_dir(&harness.config_path).await?;
    for (path, body) in [
        ("/api/settings/alerts", alert_request()),
        (
            "/api/settings/password",
            json!({"current_password": "secret", "new_password": "VeryStrong123!"}),
        ),
    ] {
        let response = harness
            .app
            .clone()
            .oneshot(json_request(path, &basic_auth_header("secret"), None, body))
            .await?;
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }
    assert_eq!(
        harness
            .state
            .readonly_auth
            .read()
            .await
            .config
            .as_ref()
            .expect("auth")
            .password,
        "secret"
    );
    assert_eq!(
        harness
            .state
            .alerting
            .read()
            .await
            .inspection
            .cpu_warn_percent,
        nodelite_proto::AlertingConfig::default()
            .inspection
            .cpu_warn_percent
    );
    tokio::fs::remove_dir(&harness.config_path).await?;
    tokio::fs::write(&harness.config_path, original).await?;
    assert_eq!(
        parse_current_config(&harness.config_path)
            .await?
            .readonly_auth
            .expect("auth")
            .password,
        "secret"
    );
    harness.cleanup().await;
    Ok(())
}

#[tokio::test]
async fn disconnected_http_caller_cannot_leave_disk_and_runtime_divergent() -> Result<()> {
    let harness = SettingsHarness::new(readonly_auth(false, None)).await?;
    let guard = harness.state.settings_write_lock.lock().await;
    let mut response = Box::pin(harness.app.clone().oneshot(json_request(
        "/api/settings/password",
        &basic_auth_header("secret"),
        None,
        json!({"current_password": "secret", "new_password": "VeryStrong123!"}),
    )));
    assert!(
        timeout(Duration::from_millis(20), &mut response)
            .await
            .is_err()
    );
    drop(response);
    drop(guard);
    // The cancelled request's transaction was queued first on the FIFO mutex.
    let committed = timeout(
        Duration::from_secs(5),
        harness.state.settings_write_lock.lock(),
    )
    .await?;
    assert_eq!(
        parse_current_config(&harness.config_path)
            .await?
            .readonly_auth
            .expect("auth")
            .password,
        "VeryStrong123!"
    );
    assert_eq!(
        harness
            .state
            .readonly_auth
            .read()
            .await
            .config
            .as_ref()
            .expect("auth")
            .password,
        "VeryStrong123!"
    );
    drop(committed);
    harness.cleanup().await;
    Ok(())
}
