//! Configuration writes keep disk and runtime changes in one serialized operation.

use anyhow::{Context, Result, anyhow, bail};
use axum::http::StatusCode;
use axum::response::Response;
use nodelite_proto::{
    AlertingConfig, ReadonlyAuthConfig, parse_server_config, upsert_toml_item_preserving_decor,
};
use serde::Serialize;
use tokio::fs;
use toml_edit::{DocumentMut, Item, Table, Value, value};

pub(super) async fn with_settings_write<F, Fut>(state: crate::AppState, operation: F) -> Response
where
    F: FnOnce(crate::AppState) -> Fut + Send + 'static,
    Fut: std::future::Future<Output = Response> + Send + 'static,
{
    // Keep the lock and runtime update alive if the HTTP caller disconnects
    // while spawn_blocking is committing the configuration file.
    match tokio::spawn(async move {
        let lock = std::sync::Arc::clone(&state.settings_write_lock);
        let _guard = lock.lock().await;
        operation(state).await
    })
    .await
    {
        Ok(response) => response,
        Err(error) => {
            tracing::error!(?error, "settings transaction task failed");
            super::settings_json_error(StatusCode::INTERNAL_SERVER_ERROR, "settings update failed")
        }
    }
}

pub(super) async fn persist_auth_password_change(
    path: &std::path::Path,
    password: &str,
) -> Result<()> {
    let content = fs::read_to_string(path)
        .await
        .with_context(|| format!("failed to read server config from {}", path.display()))?;
    let updated = update_auth_password(&content, password)?;
    validate_server_config(&updated)?;
    persist_updated_content(path, updated).await
}

pub(super) async fn persist_auth_2fa_change(
    path: &std::path::Path,
    auth: &ReadonlyAuthConfig,
) -> Result<()> {
    let content = fs::read_to_string(path)
        .await
        .with_context(|| format!("failed to read server config from {}", path.display()))?;
    let updated = update_auth_2fa(&content, auth.enable_2fa, auth.totp_secret.as_deref())?;
    validate_server_config(&updated)?;
    persist_updated_content(path, updated).await
}

pub(super) async fn persist_alerting_change(
    path: &std::path::Path,
    alerting: &AlertingConfig,
) -> Result<()> {
    let content = fs::read_to_string(path)
        .await
        .with_context(|| format!("failed to read server config from {}", path.display()))?;
    let updated = update_alerting_settings(&content, alerting)?;
    validate_server_config(&updated)?;
    persist_updated_content(path, updated).await
}

fn update_auth_password(content: &str, password: &str) -> Result<String> {
    let mut document = parse_document(content)?;
    let auth = auth_table_mut(&mut document)?;
    set_value(auth, "password", Value::from(password))?;
    Ok(document.to_string())
}

fn update_auth_2fa(content: &str, enable_2fa: bool, totp_secret: Option<&str>) -> Result<String> {
    if enable_2fa && totp_secret.is_none() {
        bail!("totp_secret is required when enabling 2FA");
    }

    let mut document = parse_document(content)?;
    let auth = auth_table_mut(&mut document)?;
    set_value(auth, "enable_2fa", Value::from(enable_2fa))?;
    match totp_secret {
        Some(secret) => set_value(auth, "totp_secret", Value::from(secret))?,
        None => {
            auth.remove("totp_secret");
        }
    }
    Ok(document.to_string())
}

fn update_alerting_settings(content: &str, alerting: &AlertingConfig) -> Result<String> {
    let mut document = parse_document(content)?;
    upsert_toml_item_preserving_decor(
        document.as_table_mut(),
        "alerts",
        build_alerts_item(alerting)?,
    );
    Ok(document.to_string())
}

fn parse_document(content: &str) -> Result<DocumentMut> {
    content
        .parse::<DocumentMut>()
        .map_err(|error| anyhow!("failed to parse server.toml as TOML document: {error}"))
}

fn auth_table_mut(document: &mut DocumentMut) -> Result<&mut Table> {
    document
        .get_mut("auth")
        .and_then(Item::as_table_mut)
        .ok_or_else(|| anyhow!("server.toml does not contain an [auth] section"))
}

fn set_value(table: &mut Table, key: &str, new_value: Value) -> Result<()> {
    if let Some(item) = table.get_mut(key) {
        let Some(existing_value) = item.as_value_mut() else {
            bail!("auth.{key} is not a value");
        };
        let decor = existing_value.decor().clone();
        *existing_value = new_value;
        *existing_value.decor_mut() = decor;
    } else {
        table.insert(key, value(new_value));
    }
    Ok(())
}

fn validate_server_config(content: &str) -> Result<()> {
    parse_server_config(content)
        .map_err(|error| anyhow!("updated server config would be invalid: {error}"))?;
    Ok(())
}

fn build_alerts_item(alerting: &AlertingConfig) -> Result<Item> {
    let fragment = toml::to_string(&AlertingDocument { alerts: alerting })
        .map_err(|error| anyhow!("failed to serialize alerts section: {error}"))?;
    let mut fragment = parse_document(&fragment)?;
    fragment
        .remove("alerts")
        .ok_or_else(|| anyhow!("serialized alerting config did not produce an [alerts] section"))
}

async fn persist_updated_content(path: &std::path::Path, updated: String) -> Result<()> {
    let path = path.to_path_buf();
    tokio::task::spawn_blocking(move || {
        crate::fs_security::atomic_write_private(&path, updated.as_bytes())
    })
    .await
    .context("configuration write task failed")?
    .context("failed to commit server configuration")
}

#[derive(Serialize)]
struct AlertingDocument<'a> {
    alerts: &'a AlertingConfig,
}

#[cfg(test)]
#[path = "config_edit_tests.rs"]
mod tests;
