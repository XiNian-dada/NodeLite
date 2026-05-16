use std::path::{Path, PathBuf};

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use getrandom::fill as fill_random;
use nodelite_proto::{ReadonlyAuthConfig, parse_server_config};
use tokio::fs;

use super::SettingsActionResponse;

pub(super) fn settings_json_error(status: StatusCode, message: impl Into<String>) -> Response {
    (
        status,
        Json(SettingsActionResponse {
            ok: false,
            message: message.into(),
        }),
    )
        .into_response()
}

pub(super) fn validate_password_for_settings(password: &str) -> Result<(), &'static str> {
    const MAX_PASSWORD_CHARS: usize = 128;

    if password.len() < 8 {
        return Err("new password must be at least 8 characters");
    }
    if password.chars().count() > MAX_PASSWORD_CHARS {
        return Err("new password must be at most 128 characters");
    }
    if !password.chars().any(|c| c.is_alphabetic()) || !password.chars().any(|c| c.is_ascii_digit())
    {
        return Err("new password must include both letters and digits");
    }
    Ok(())
}

pub(super) fn server_update_log_path(config: &nodelite_proto::ServerConfig) -> PathBuf {
    let base_dir = config
        .snapshot_path
        .parent()
        .or_else(|| config.history_db_path.parent())
        .or_else(|| config.node_registry_path.parent())
        .unwrap_or_else(|| Path::new("/tmp"));
    base_dir.join("server-web-update.log")
}

pub(super) fn server_update_shell_command(log_path: &Path) -> String {
    let installer_url = format!(
        "{}/releases/latest/download/install-server.sh",
        env!("CARGO_PKG_REPOSITORY")
    );
    [
        "set -u".to_string(),
        format!("log={}", shell_quote(&log_path.display().to_string())),
        "tmp_script=\"$(mktemp /tmp/nodelite-install-server.XXXXXX)\"".to_string(),
        "trap 'rm -f \"$tmp_script\"' EXIT".to_string(),
        ": >\"$log\"".to_string(),
        "echo \"nodelite-update: started at $(date -u +%Y-%m-%dT%H:%M:%SZ)\" >>\"$log\"".to_string(),
        format!(
            "echo \"nodelite-update: downloading {}\" >>\"$log\"",
            shell_quote(&installer_url)
        ),
        format!(
            "curl -fsSL {} -o \"$tmp_script\" >>\"$log\" 2>&1",
            shell_quote(&installer_url)
        ),
        "download_status=$?".to_string(),
        "if [ \"$download_status\" -ne 0 ]; then echo \"nodelite-update: finished exit=$download_status at $(date -u +%Y-%m-%dT%H:%M:%SZ)\" >>\"$log\"; exit \"$download_status\"; fi".to_string(),
        "chmod 0700 \"$tmp_script\" >>\"$log\" 2>&1".to_string(),
        "chmod_status=$?".to_string(),
        "if [ \"$chmod_status\" -ne 0 ]; then echo \"nodelite-update: finished exit=$chmod_status at $(date -u +%Y-%m-%dT%H:%M:%SZ)\" >>\"$log\"; exit \"$chmod_status\"; fi".to_string(),
        "echo \"nodelite-update: running installer in upgrade mode\" >>\"$log\"".to_string(),
        format!(
            "NODELITE_SERVER_MODE=upgrade sh \"$tmp_script\" >>\"$log\" 2>&1; update_status=$?; echo \"nodelite-update: finished exit=$update_status at $(date -u +%Y-%m-%dT%H:%M:%SZ)\" >>\"$log\"; exit \"$update_status\" # {}",
            shell_quote(&installer_url)
        ),
    ]
    .join("\n")
}

pub(super) async fn persist_auth_password_change(
    path: &std::path::Path,
    password: &str,
) -> anyhow::Result<()> {
    let content = fs::read_to_string(path).await?;
    let updated = replace_auth_password(&content, password)?;
    parse_server_config(&updated)
        .map_err(|error| anyhow::anyhow!("updated server config would be invalid: {error}"))?;
    let metadata = fs::metadata(path).await.ok();
    let temp_path = path.with_extension("toml.tmp");
    fs::write(&temp_path, updated).await?;
    if let Some(metadata) = metadata {
        fs::set_permissions(&temp_path, metadata.permissions()).await?;
    }
    fs::rename(&temp_path, path).await?;
    Ok(())
}

pub(super) async fn persist_auth_2fa_change(
    path: &std::path::Path,
    auth: &ReadonlyAuthConfig,
) -> anyhow::Result<()> {
    let content = fs::read_to_string(path).await?;
    let updated = replace_auth_2fa(&content, auth.enable_2fa, auth.totp_secret.as_deref())?;
    parse_server_config(&updated)
        .map_err(|error| anyhow::anyhow!("updated server config would be invalid: {error}"))?;
    let metadata = fs::metadata(path).await.ok();
    let temp_path = path.with_extension("toml.tmp");
    fs::write(&temp_path, updated).await?;
    if let Some(metadata) = metadata {
        fs::set_permissions(&temp_path, metadata.permissions()).await?;
    }
    fs::rename(&temp_path, path).await?;
    Ok(())
}

pub(super) fn generate_totp_secret() -> anyhow::Result<String> {
    let mut bytes = [0_u8; 20];
    fill_random(&mut bytes)?;
    Ok(base32::encode(
        base32::Alphabet::Rfc4648 { padding: false },
        &bytes,
    ))
}

pub(super) fn otpauth_uri(username: &str, secret: &str) -> String {
    let issuer = "NodeLite";
    format!(
        "otpauth://totp/{}:{}?secret={}&issuer={}",
        percent_encode_component(issuer),
        percent_encode_component(username),
        percent_encode_component(secret),
        percent_encode_component(issuer)
    )
}

pub(super) fn server_build_version() -> &'static str {
    option_env!("NODELITE_BUILD_VERSION").unwrap_or(env!("CARGO_PKG_VERSION"))
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn replace_auth_password(content: &str, password: &str) -> anyhow::Result<String> {
    let escaped_password = toml_basic_string(password);
    let mut output = Vec::new();
    let mut in_auth = false;
    let mut seen_auth = false;
    let mut replaced = false;

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            if in_auth && !replaced {
                output.push(format!("password = \"{escaped_password}\""));
                replaced = true;
            }
            in_auth = trimmed == "[auth]";
            seen_auth |= in_auth;
        }

        if in_auth && is_toml_key(trimmed, "password") {
            let indent = &line[..line.len() - line.trim_start().len()];
            output.push(format!("{indent}password = \"{escaped_password}\""));
            replaced = true;
            continue;
        }
        output.push(line.to_string());
    }

    if !seen_auth {
        anyhow::bail!("server.toml does not contain an [auth] section");
    }
    if in_auth && !replaced {
        output.push(format!("password = \"{escaped_password}\""));
    }
    Ok(format!("{}\n", output.join("\n")))
}

fn replace_auth_2fa(
    content: &str,
    enable_2fa: bool,
    totp_secret: Option<&str>,
) -> anyhow::Result<String> {
    if enable_2fa && totp_secret.is_none() {
        anyhow::bail!("totp_secret is required when enabling 2FA");
    }

    let mut output = Vec::new();
    let mut in_auth = false;
    let mut seen_auth = false;
    let mut wrote_enable = false;
    let mut wrote_secret = false;

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            if in_auth {
                write_missing_2fa_lines(
                    &mut output,
                    enable_2fa,
                    totp_secret,
                    &mut wrote_enable,
                    &mut wrote_secret,
                );
            }
            in_auth = trimmed == "[auth]";
            seen_auth |= in_auth;
        }

        if in_auth && is_toml_key(trimmed, "enable_2fa") {
            let indent = &line[..line.len() - line.trim_start().len()];
            output.push(format!("{indent}enable_2fa = {enable_2fa}"));
            wrote_enable = true;
            continue;
        }
        if in_auth && is_toml_key(trimmed, "totp_secret") {
            if let Some(secret) = totp_secret {
                let indent = &line[..line.len() - line.trim_start().len()];
                output.push(format!(
                    "{indent}totp_secret = \"{}\"",
                    toml_basic_string(secret)
                ));
                wrote_secret = true;
            }
            continue;
        }
        output.push(line.to_string());
    }

    if !seen_auth {
        anyhow::bail!("server.toml does not contain an [auth] section");
    }
    if in_auth {
        write_missing_2fa_lines(
            &mut output,
            enable_2fa,
            totp_secret,
            &mut wrote_enable,
            &mut wrote_secret,
        );
    }
    Ok(format!("{}\n", output.join("\n")))
}

fn write_missing_2fa_lines(
    output: &mut Vec<String>,
    enable_2fa: bool,
    totp_secret: Option<&str>,
    wrote_enable: &mut bool,
    wrote_secret: &mut bool,
) {
    if !*wrote_enable {
        output.push(format!("enable_2fa = {enable_2fa}"));
        *wrote_enable = true;
    }
    if let Some(secret) = totp_secret
        && !*wrote_secret
    {
        output.push(format!("totp_secret = \"{}\"", toml_basic_string(secret)));
        *wrote_secret = true;
    }
}

fn percent_encode_component(value: &str) -> String {
    let mut output = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            output.push(byte as char);
        } else {
            output.push_str(&format!("%{byte:02X}"));
        }
    }
    output
}

fn is_toml_key(trimmed: &str, key: &str) -> bool {
    trimmed
        .strip_prefix(key)
        .is_some_and(|rest| rest.trim_start().starts_with('='))
}

fn toml_basic_string(value: &str) -> String {
    let mut output = String::new();
    for ch in value.chars() {
        match ch {
            '\\' => output.push_str("\\\\"),
            '"' => output.push_str("\\\""),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            ch if ch.is_control() => {
                output.push_str(&format!("\\u{:04x}", ch as u32));
            }
            ch => output.push(ch),
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::{otpauth_uri, replace_auth_2fa, validate_password_for_settings};

    #[test]
    fn replace_auth_2fa_enables_and_preserves_auth_section() {
        let input = r#"[server]
listen = "127.0.0.1:8080"
public_base_url = "https://monitor.example.com"

[auth]
username = "viewer"
password = "old-pass"

[ui]
refresh_interval_secs = 5
"#;

        let updated = replace_auth_2fa(input, true, Some("JBSWY3DPEHPK3PXP"))
            .expect("2FA enable should update auth section");

        assert!(updated.contains("username = \"viewer\""));
        assert!(updated.contains("password = \"old-pass\""));
        assert!(updated.contains("enable_2fa = true"));
        assert!(updated.contains("totp_secret = \"JBSWY3DPEHPK3PXP\""));
        assert!(updated.contains("[ui]"));
    }

    #[test]
    fn replace_auth_2fa_disables_and_removes_stale_secret() {
        let input = r#"[auth]
username = "viewer"
password = "old-pass"
enable_2fa = true
totp_secret = "JBSWY3DPEHPK3PXP"
"#;

        let updated =
            replace_auth_2fa(input, false, None).expect("2FA disable should update auth section");

        assert!(updated.contains("enable_2fa = false"));
        assert!(!updated.contains("totp_secret"));
    }

    #[test]
    fn otpauth_uri_percent_encodes_account_label() {
        let uri = otpauth_uri("viewer@example.com", "JBSWY3DPEHPK3PXP");

        assert_eq!(
            uri,
            "otpauth://totp/NodeLite:viewer%40example.com?secret=JBSWY3DPEHPK3PXP&issuer=NodeLite"
        );
    }

    #[test]
    fn validate_password_for_settings_rejects_overlong_passwords() {
        let password = format!("Aa1{}", "x".repeat(130));
        assert_eq!(
            validate_password_for_settings(&password),
            Err("new password must be at most 128 characters")
        );
    }
}
