use std::path::{Path, PathBuf};

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
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
    const MIN_PASSWORD_CHARS: usize = 12;
    const MAX_PASSWORD_CHARS: usize = 128;

    // 长度检查
    if password.len() < MIN_PASSWORD_CHARS {
        return Err("new password must be at least 12 characters");
    }
    if password.chars().count() > MAX_PASSWORD_CHARS {
        return Err("new password must be at most 128 characters");
    }

    // 字符类型检查
    let has_upper = password.chars().any(|c| c.is_uppercase());
    let has_lower = password.chars().any(|c| c.is_lowercase());
    let has_digit = password.chars().any(|c| c.is_ascii_digit());
    let has_special = password.chars().any(|c| !c.is_alphanumeric());

    if !has_upper {
        return Err("new password must include at least one uppercase letter");
    }
    if !has_lower {
        return Err("new password must include at least one lowercase letter");
    }
    if !has_digit {
        return Err("new password must include at least one digit");
    }
    if !has_special {
        return Err("new password must include at least one special character");
    }

    // 常见密码检查
    if is_common_password(password) {
        return Err("new password is too common, please choose a stronger password");
    }

    Ok(())
}

/// 检查是否为常见弱密码
fn is_common_password(password: &str) -> bool {
    // Top 100 最常见密码列表（小写）
    const COMMON_PASSWORDS: &[&str] = &[
        "password",
        "password1",
        "password123",
        "password12",
        "password1234",
        "123456",
        "12345678",
        "123456789",
        "1234567890",
        "qwerty",
        "abc123",
        "monkey",
        "1234567",
        "letmein",
        "trustno1",
        "dragon",
        "baseball",
        "iloveyou",
        "master",
        "sunshine",
        "ashley",
        "bailey",
        "passw0rd",
        "shadow",
        "123123",
        "654321",
        "superman",
        "qazwsx",
        "michael",
        "football",
        "welcome",
        "jesus",
        "ninja",
        "mustang",
        "password!",
        "admin",
        "admin123",
        "root",
        "toor",
        "pass",
        "test",
        "guest",
        "info",
        "adm",
        "mysql",
        "user",
        "administrator",
        "oracle",
        "ftp",
        "pi",
        "puppet",
        "ansible",
        "ec2-user",
        "vagrant",
        "azureuser",
        "changeme",
        "changeme123",
        "default",
        "password@123",
        "p@ssw0rd",
        "p@ssword",
        "passw0rd!",
        "admin@123",
        "root123",
        "test123",
        "demo",
        "demo123",
        "sample",
        "temp",
        "temp123",
        "nodelite",
        "monitor",
        "monitoring",
        "server",
        "agent",
        "qwerty123",
        "abc123456",
        "password!",
        "letmein123",
        "welcome123",
        "123qwe",
        "qwe123",
        "1q2w3e4r",
        "1qaz2wsx",
        "zxcvbnm",
        "asdfgh",
        "qwertyuiop",
        "1234qwer",
        "qwer1234",
        "abcd1234",
        "password1!",
        "password123!",
        "admin1234",
        "root1234",
        "pass1234",
        "test1234",
        "demo1234",
        "temp1234",
        "user1234",
        "welcome1",
        "admin123!@#",
        "admin123!@#$",
        "welcome123!@",
    ];

    let lower = password.to_lowercase();

    // 检查完全匹配
    if COMMON_PASSWORDS.contains(&lower.as_str()) {
        return true;
    }

    // 检查是否包含常见密码作为子串（去除特殊字符后）
    let alphanumeric: String = lower.chars().filter(|c| c.is_alphanumeric()).collect();
    for common in COMMON_PASSWORDS {
        let common_alphanum: String = common.chars().filter(|c| c.is_alphanumeric()).collect();
        if !common_alphanum.is_empty() && alphanumeric == common_alphanum {
            return true;
        }
    }

    false
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

pub(super) fn server_update_cache_dir(config: &nodelite_proto::ServerConfig) -> PathBuf {
    let base_dir = server_update_log_path(config)
        .parent()
        .unwrap_or_else(|| Path::new("/tmp"))
        .to_path_buf();
    base_dir.join(".server-update-cache")
}

pub(super) fn server_update_writable_paths(config: &nodelite_proto::ServerConfig) -> Vec<PathBuf> {
    let install_root = server_update_install_root(config);
    let log_dir = server_update_log_path(config)
        .parent()
        .unwrap_or_else(|| Path::new("/tmp"))
        .to_path_buf();
    let cache_dir = server_update_cache_dir(config);

    let mut paths = Vec::new();
    for candidate in [
        install_root,
        log_dir,
        cache_dir,
        PathBuf::from("/usr/local/bin"),
        PathBuf::from("/etc/systemd/system"),
    ] {
        if !paths.iter().any(|existing| existing == &candidate) {
            paths.push(candidate);
        }
    }
    paths
}

pub(super) fn server_update_shell_command(log_path: &Path, cache_dir: &Path) -> String {
    let installer_url = format!(
        "{}/releases/latest/download/install-server.sh",
        env!("CARGO_PKG_REPOSITORY")
    );
    [
        "set -u".to_string(),
        "umask 077".to_string(),
        format!("log={}", shell_quote(&log_path.display().to_string())),
        format!(
            "cache_dir={}",
            shell_quote(&cache_dir.display().to_string())
        ),
        "mkdir -p \"$cache_dir\"".to_string(),
        "chmod 0700 \"$cache_dir\" >>\"$log\" 2>&1 || true".to_string(),
        "tmp_script=\"$(mktemp \"$cache_dir/install-server.XXXXXX\")\"".to_string(),
        "trap 'rm -f \"$tmp_script\"' EXIT".to_string(),
        "chmod 0600 \"$tmp_script\" >>\"$log\" 2>&1".to_string(),
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

fn server_update_install_root(config: &nodelite_proto::ServerConfig) -> PathBuf {
    let all_paths: Vec<&Path> = [
        config.node_registry_path.parent(),
        config.history_db_path.parent(),
        config.snapshot_path.parent(),
    ]
    .into_iter()
    .flatten()
    .collect();
    let mut current = all_paths
        .first()
        .copied()
        .unwrap_or_else(|| Path::new("/tmp"))
        .to_path_buf();
    while !all_paths.iter().all(|path| path.starts_with(&current)) {
        let Some(parent) = current.parent() else {
            return PathBuf::from("/tmp");
        };
        current = parent.to_path_buf();
    }
    current
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
    use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
    use std::path::{Path, PathBuf};

    use super::{
        otpauth_uri, replace_auth_2fa, server_update_cache_dir, server_update_shell_command,
        server_update_writable_paths, validate_password_for_settings,
    };
    use nodelite_proto::{ServerConfig, WsConfig};

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
        let password = format!("Aa1!{}", "x".repeat(130));
        assert_eq!(
            validate_password_for_settings(&password),
            Err("new password must be at most 128 characters")
        );
    }

    #[test]
    fn validate_password_for_settings_rejects_short_passwords() {
        assert_eq!(
            validate_password_for_settings("Short1!"),
            Err("new password must be at least 12 characters")
        );
    }

    #[test]
    fn validate_password_for_settings_requires_uppercase() {
        assert_eq!(
            validate_password_for_settings("lowercase123!"),
            Err("new password must include at least one uppercase letter")
        );
    }

    #[test]
    fn validate_password_for_settings_requires_lowercase() {
        assert_eq!(
            validate_password_for_settings("UPPERCASE123!"),
            Err("new password must include at least one lowercase letter")
        );
    }

    #[test]
    fn validate_password_for_settings_requires_digit() {
        assert_eq!(
            validate_password_for_settings("NoDigitsHere!"),
            Err("new password must include at least one digit")
        );
    }

    #[test]
    fn validate_password_for_settings_requires_special_char() {
        assert_eq!(
            validate_password_for_settings("NoSpecial123"),
            Err("new password must include at least one special character")
        );
    }

    #[test]
    fn validate_password_for_settings_rejects_common_passwords() {
        // 使用常见密码列表中的密码，但满足长度和复杂度要求
        assert_eq!(
            validate_password_for_settings("Password123!"),
            Err("new password is too common, please choose a stronger password")
        );
        assert_eq!(
            validate_password_for_settings("Admin123!@#$"),
            Err("new password is too common, please choose a stronger password")
        );
        assert_eq!(
            validate_password_for_settings("Welcome123!@"),
            Err("new password is too common, please choose a stronger password")
        );
    }

    #[test]
    fn validate_password_for_settings_accepts_strong_passwords() {
        assert!(validate_password_for_settings("MyStr0ng!Pass").is_ok());
        assert!(validate_password_for_settings("C0mpl3x@Passw0rd!").is_ok());
        assert!(validate_password_for_settings("Secure#2024$Pass").is_ok());
    }

    #[test]
    fn server_update_paths_stay_under_install_root_plus_required_system_dirs() {
        let config = sample_server_config();
        let paths = server_update_writable_paths(&config);

        assert!(paths.contains(&PathBuf::from("/opt/nodelite")));
        assert!(paths.contains(&PathBuf::from("/opt/nodelite/data")));
        assert!(paths.contains(&PathBuf::from("/opt/nodelite/data/.server-update-cache")));
        assert!(paths.contains(&PathBuf::from("/usr/local/bin")));
        assert!(paths.contains(&PathBuf::from("/etc/systemd/system")));
    }

    #[test]
    fn server_update_shell_command_uses_private_cache_dir_for_temp_script() {
        let config = sample_server_config();
        let log_path = Path::new("/tmp/nodelite-update.log");
        let cache_dir = server_update_cache_dir(&config);
        let command = server_update_shell_command(log_path, &cache_dir);

        assert!(command.contains("umask 077"));
        assert!(command.contains("cache_dir='/opt/nodelite/data/.server-update-cache'"));
        assert!(command.contains("mkdir -p \"$cache_dir\""));
        assert!(command.contains("tmp_script=\"$(mktemp \"$cache_dir/install-server.XXXXXX\")\""));
        assert!(command.contains("chmod 0600 \"$tmp_script\""));
        assert!(!command.contains("${HOME:-/tmp}/.cache"));
    }

    fn sample_server_config() -> ServerConfig {
        ServerConfig {
            listen: SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 8080)),
            public_base_url: "https://example.com".to_string(),
            insecure_allow_http: false,
            readonly_auth: None,
            ws: WsConfig {
                max_total_connections: 32,
                max_connections_per_ip: 32,
                auth_fail_window_secs: 300,
                auth_fail_max_attempts: 8,
                auth_block_secs: 900,
            },
            node_registry_path: PathBuf::from("/opt/nodelite/config/server.json"),
            history_db_path: PathBuf::from("/opt/nodelite/data/history.sqlite3"),
            snapshot_path: PathBuf::from("/opt/nodelite/data/snapshot.json"),
            stale_after_secs: 15,
            ping_interval_secs: 60,
            max_message_bytes: 64 * 1024,
            refresh_interval_secs: 5,
            ignored_filesystems: Vec::new(),
            agent_release_base_url: None,
            agent_release_sha256_x86_64: None,
            agent_release_sha256_aarch64: None,
            hello_timeout_secs: 10,
            max_outstanding_pings: 32,
            insecure_transport_warn_interval_secs: 900,
            max_sanitized_disks: 64,
            max_sanitized_string_bytes: 256,
            metric_anomaly_session_limit: 5,
            sqlite_busy_timeout_secs: 5,
        }
    }
}
