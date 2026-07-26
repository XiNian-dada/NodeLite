use std::path::{Component, Path, PathBuf};

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use getrandom::fill as fill_random;
use thiserror::Error;

use crate::encoding::shell_quote;

use super::SettingsActionResponse;

#[derive(Debug, Error)]
pub(super) enum SettingsHelperError {
    #[error("failed to gather secure random bytes for TOTP secret")]
    Random(#[from] getrandom::Error),
}

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
    crate::auth::validate_password_strength(password)
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

pub(super) fn is_writable_paths_subset_of_install_root(
    paths: &[PathBuf],
    install_root: &Path,
) -> bool {
    paths.iter().all(|path| {
        !path
            .components()
            .any(|component| component == Component::ParentDir)
            && (path.starts_with(install_root)
                || path == Path::new("/usr/local/bin")
                || path.starts_with("/usr/local/bin/")
                || path == Path::new("/etc/systemd/system")
                || path.starts_with("/etc/systemd/system/"))
    })
}

pub(super) fn server_update_shell_command(log_path: &Path, cache_dir: &Path) -> String {
    [
        format!(
            "NODELITE_UPDATE_LOG={}",
            shell_quote(&log_path.display().to_string())
        ),
        format!(
            "NODELITE_UPDATE_CACHE_DIR={}",
            shell_quote(&cache_dir.display().to_string())
        ),
        format!(
            "NODELITE_UPDATE_REPOSITORY={}",
            shell_quote(env!("CARGO_PKG_REPOSITORY"))
        ),
        include_str!("../../../../scripts/server-web-update.sh").to_string(),
    ]
    .join("\n")
}

pub(super) fn server_update_install_root(config: &nodelite_proto::ServerConfig) -> PathBuf {
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

pub(super) fn generate_totp_secret() -> Result<String, SettingsHelperError> {
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

#[cfg(test)]
mod tests {
    use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
    use std::path::{Path, PathBuf};

    use nodelite_proto::{ServerConfig, WsConfig};

    use super::{
        is_writable_paths_subset_of_install_root, otpauth_uri, server_update_cache_dir,
        server_update_install_root, server_update_shell_command, server_update_writable_paths,
        validate_password_for_settings,
    };

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
            Err("password must be at most 128 characters")
        );
    }

    #[test]
    fn validate_password_for_settings_rejects_short_passwords() {
        assert_eq!(
            validate_password_for_settings("Short1!"),
            Err("password must be at least 12 characters")
        );
    }

    #[test]
    fn validate_password_for_settings_requires_uppercase() {
        assert_eq!(
            validate_password_for_settings("lowercase123!"),
            Err("password must include at least one uppercase letter")
        );
    }

    #[test]
    fn validate_password_for_settings_requires_lowercase() {
        assert_eq!(
            validate_password_for_settings("UPPERCASE123!"),
            Err("password must include at least one lowercase letter")
        );
    }

    #[test]
    fn validate_password_for_settings_requires_digit() {
        assert_eq!(
            validate_password_for_settings("NoDigitsHere!"),
            Err("password must include at least one digit")
        );
    }

    #[test]
    fn validate_password_for_settings_requires_special_char() {
        assert_eq!(
            validate_password_for_settings("NoSpecial123"),
            Err("password must include at least one special character")
        );
    }

    #[test]
    fn validate_password_for_settings_rejects_common_passwords() {
        assert_eq!(
            validate_password_for_settings("Password123!"),
            Err("password is too common, please choose a stronger password")
        );
        assert_eq!(
            validate_password_for_settings("Admin123!@#$"),
            Err("password is too common, please choose a stronger password")
        );
        assert_eq!(
            validate_password_for_settings("Welcome123!@"),
            Err("password is too common, please choose a stronger password")
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
        let install_root = server_update_install_root(&config);

        assert!(paths.contains(&PathBuf::from("/opt/nodelite")));
        assert!(paths.contains(&PathBuf::from("/opt/nodelite/data")));
        assert!(paths.contains(&PathBuf::from("/opt/nodelite/data/.server-update-cache")));
        assert!(paths.contains(&PathBuf::from("/usr/local/bin")));
        assert!(paths.contains(&PathBuf::from("/etc/systemd/system")));
        assert!(is_writable_paths_subset_of_install_root(
            &paths,
            &install_root,
        ));
    }

    #[test]
    fn server_update_paths_reject_parent_dir_and_unapproved_roots() {
        let install_root = Path::new("/opt/nodelite");
        let invalid_root = vec![
            PathBuf::from("/opt/nodelite/data"),
            PathBuf::from("/var/lib/nodelite"),
        ];
        assert!(!is_writable_paths_subset_of_install_root(
            &invalid_root,
            install_root,
        ));

        let parent_dir_escape = vec![PathBuf::from("/usr/local/bin/../etc")];
        assert!(!is_writable_paths_subset_of_install_root(
            &parent_dir_escape,
            install_root,
        ));
    }

    #[test]
    fn server_update_shell_command_embeds_versioned_verified_bootstrap() {
        let config = sample_server_config();
        let log_path = Path::new("/tmp/nodelite-update.log");
        let cache_dir = server_update_cache_dir(&config);
        let command = server_update_shell_command(log_path, &cache_dir);

        assert!(command.contains("umask 077"));
        assert!(
            command.contains("NODELITE_UPDATE_CACHE_DIR='/opt/nodelite/data/.server-update-cache'")
        );
        assert!(command.contains("mkdir -p \"$cache_dir\""));
        assert!(command.contains("tmp_script=\"$(mktemp \"$cache_dir/install-server.XXXXXX\")\""));
        assert!(command.contains("chmod 0600 \"$tmp_script\""));
        assert!(command.contains("asset_base_url=\"$repository/releases/download/$target_tag\""));
        assert!(command.contains("installer sha256=$actual_sha256 verified"));
        assert!(command.contains("NODELITE_SERVER_VERSION=\"$target_tag\""));
        assert!(command.contains("NODELITE_SERVER_BASE_URL=\"$asset_base_url\""));
        assert!(!command.contains("releases/latest/download/install-server.sh"));
        assert!(!command.contains("${HOME:-/tmp}/.cache"));

        let verify_position = command
            .find("installer sha256=$actual_sha256 verified")
            .expect("checksum verification log should be present");
        let execute_position = command
            .find("sh \"$tmp_script\"")
            .expect("installer execution should be present");
        assert!(verify_position < execute_position);
    }

    fn sample_server_config() -> ServerConfig {
        ServerConfig {
            listen: SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 8080)),
            public_base_url: "https://example.com".to_string(),
            insecure_allow_http: false,
            trusted_proxies: Vec::new(),
            readonly_auth: None,
            ws: WsConfig {
                max_total_connections: 32,
                max_connections_per_ip: 32,
                auth_fail_window_secs: 300,
                auth_fail_max_attempts: 8,
                auth_block_secs: 900,
            },
            metrics: nodelite_proto::MetricsConfig::default(),
            audit: nodelite_proto::AuditConfig {
                enabled: true,
                db_path: PathBuf::from("/opt/nodelite/data/audit.sqlite3"),
                retention_days: 90,
                writer_batch_max: nodelite_proto::DEFAULT_AUDIT_WRITER_BATCH_MAX,
                writer_flush_interval_ms: nodelite_proto::DEFAULT_AUDIT_WRITER_FLUSH_INTERVAL_MS,
                log_successful_auth: true,
                log_failed_auth: true,
                log_token_events: true,
                log_rate_limit: true,
            },
            geoip: nodelite_proto::GeoIpConfig {
                enabled: false,
                provider: nodelite_proto::GeoIpProvider::Dbip,
                edition: nodelite_proto::GeoIpEdition::CountryLite,
                database_path: PathBuf::from("./data/geoip/dbip.mmdb"),
                auto_update: true,
                update_interval_days: nodelite_proto::DEFAULT_GEOIP_UPDATE_INTERVAL_DAYS,
            },
            alerting: nodelite_proto::AlertingConfig::default(),
            node_registry_path: PathBuf::from("/opt/nodelite/config/server.json"),
            history_db_path: PathBuf::from("/opt/nodelite/data/history.sqlite3"),
            history_query_concurrency: nodelite_proto::DEFAULT_HISTORY_QUERY_CONCURRENCY,
            history_read_cache_kib: nodelite_proto::DEFAULT_HISTORY_READ_CACHE_KIB,
            history_writer_batch_max: nodelite_proto::DEFAULT_HISTORY_WRITER_BATCH_MAX,
            history_writer_flush_interval_ms:
                nodelite_proto::DEFAULT_HISTORY_WRITER_FLUSH_INTERVAL_MS,
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
            token_verify_max_parallelism: nodelite_proto::DEFAULT_TOKEN_VERIFY_MAX_PARALLELISM,
        }
    }
}
