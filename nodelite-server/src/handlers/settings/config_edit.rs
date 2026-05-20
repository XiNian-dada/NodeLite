use anyhow::{Result, anyhow, bail};
use nodelite_proto::{ReadonlyAuthConfig, parse_server_config};
use tokio::fs;
use toml_edit::{DocumentMut, Item, Table, Value, value};

pub(super) async fn persist_auth_password_change(
    path: &std::path::Path,
    password: &str,
) -> Result<()> {
    let content = fs::read_to_string(path).await?;
    let updated = update_auth_password(&content, password)?;
    validate_server_config(&updated)?;
    persist_updated_content(path, updated).await
}

pub(super) async fn persist_auth_2fa_change(
    path: &std::path::Path,
    auth: &ReadonlyAuthConfig,
) -> Result<()> {
    let content = fs::read_to_string(path).await?;
    let updated = update_auth_2fa(&content, auth.enable_2fa, auth.totp_secret.as_deref())?;
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

async fn persist_updated_content(path: &std::path::Path, updated: String) -> Result<()> {
    let metadata = fs::metadata(path).await.ok();
    let temp_path = path.with_extension("toml.tmp");
    fs::write(&temp_path, updated).await?;
    if let Some(metadata) = metadata {
        fs::set_permissions(&temp_path, metadata.permissions()).await?;
    }
    fs::rename(&temp_path, path).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{update_auth_2fa, update_auth_password};

    #[test]
    fn update_auth_password_preserves_trailing_comment_and_multiline_neighbors() {
        let input = r#"[server]
listen = "127.0.0.1:8080"
public_base_url = "https://monitor.example.com"

[auth]
username = "viewer"
password = "old-pass" # keep this comment

[ui]
welcome = """
hello
world
"""
"#;

        let updated = update_auth_password(input, "new-pass")
            .expect("password change should preserve neighboring TOML");

        assert!(updated.contains(r#"password = "new-pass" # keep this comment"#));
        assert!(updated.contains("welcome = \"\"\"\nhello\nworld\n\"\"\""));
    }

    #[test]
    fn update_auth_2fa_enables_and_preserves_auth_section() {
        let input = r#"[server]
listen = "127.0.0.1:8080"
public_base_url = "https://monitor.example.com"

[auth]
username = "viewer"
password = "old-pass"

[ui]
refresh_interval_secs = 5
"#;

        let updated = update_auth_2fa(input, true, Some("JBSWY3DPEHPK3PXP"))
            .expect("2FA enable should update auth section");

        assert!(updated.contains("username = \"viewer\""));
        assert!(updated.contains("password = \"old-pass\""));
        assert!(updated.contains("enable_2fa = true"));
        assert!(updated.contains("totp_secret = \"JBSWY3DPEHPK3PXP\""));
        assert!(updated.contains("[ui]"));
    }

    #[test]
    fn update_auth_2fa_disables_and_removes_stale_secret() {
        let input = r#"[auth]
username = "viewer"
password = "old-pass"
enable_2fa = true
totp_secret = "JBSWY3DPEHPK3PXP" # stale
"#;

        let updated =
            update_auth_2fa(input, false, None).expect("2FA disable should update auth section");

        assert!(updated.contains("enable_2fa = false"));
        assert!(!updated.contains("totp_secret"));
    }

    #[test]
    fn update_auth_password_rejects_missing_auth_section() {
        let input = r#"[server]
listen = "127.0.0.1:8080"
"#;

        let error = update_auth_password(input, "new-pass").expect_err("missing auth should fail");
        assert!(error.to_string().contains("[auth] section"));
    }
}
