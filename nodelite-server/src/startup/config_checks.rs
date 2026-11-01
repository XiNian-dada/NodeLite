//! Startup warnings for config-file hygiene that parsing alone cannot see.

use std::path::{Path, PathBuf};

use nodelite_proto::ServerConfig;
use tracing::warn;

/// server.toml 存有面板密码、TOTP 与告警凭据，组/其他用户可读即视为泄露面。
pub(super) fn warn_if_config_file_is_not_private(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        match std::fs::metadata(path) {
            Ok(metadata) => {
                let mode = metadata.permissions().mode() & 0o777;
                if mode_is_too_broad(mode) {
                    warn!(
                        path = %path.display(),
                        mode = format!("{mode:o}"),
                        "server config file is readable or writable by group/others; run chmod 600 on it",
                    );
                }
            }
            Err(error) => warn!(
                path = %path.display(),
                error = ?error,
                "failed to inspect server config file permissions",
            ),
        }
    }

    #[cfg(not(unix))]
    {
        let _ = path;
    }
}

fn mode_is_too_broad(mode: u32) -> bool {
    mode & 0o077 != 0
}

/// 相对路径按进程工作目录解析（与已有部署和升级脚本一致），CLI 与 systemd 服务
/// 工作目录不同时会读写不同文件，因此逐个提示实际解析位置。
pub(super) fn warn_about_relative_data_paths(config: &ServerConfig) {
    let working_dir = std::env::current_dir().ok();
    for (field, path) in relative_data_paths(config) {
        let resolved = working_dir
            .as_deref()
            .map(|dir| dir.join(path))
            .unwrap_or_else(|| path.to_path_buf());
        warn!(
            field,
            path = %path.display(),
            resolved = %resolved.display(),
            "relative data path resolves against the current working directory; use an absolute path so CLI commands and the service share the same file",
        );
    }
}

fn relative_data_paths(config: &ServerConfig) -> Vec<(&'static str, &Path)> {
    let mut candidates: Vec<(&'static str, &PathBuf)> = vec![
        ("server.node_registry_path", &config.node_registry_path),
        ("server.history_db_path", &config.history_db_path),
        ("server.snapshot_path", &config.snapshot_path),
    ];
    if config.audit.enabled {
        candidates.push(("audit.db_path", &config.audit.db_path));
    }
    if config.geoip.enabled {
        candidates.push(("geoip.database_path", &config.geoip.database_path));
    }
    candidates
        .into_iter()
        .filter(|(_, path)| path.is_relative())
        .map(|(field, path)| (field, path.as_path()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: &str = r#"
        [server]
        listen = "127.0.0.1:8080"
        public_base_url = "http://127.0.0.1:8080"
        node_registry_path = "/var/lib/nodelite/server.json"
        history_db_path = "./data/history.sqlite3"
        snapshot_path = "/var/lib/nodelite/snapshot.json"

        [audit]
        enabled = true
        db_path = "data/audit.sqlite3"

        [geoip]
        enabled = false
    "#;

    #[test]
    fn relative_data_paths_lists_only_relative_enabled_paths() {
        let config = nodelite_proto::parse_server_config(BASE).expect("config should parse");
        let fields: Vec<_> = relative_data_paths(&config)
            .into_iter()
            .map(|(field, _)| field)
            .collect();
        assert_eq!(fields, vec!["server.history_db_path", "audit.db_path"]);
    }

    #[test]
    fn group_or_world_bits_count_as_too_broad() {
        assert!(!mode_is_too_broad(0o600));
        assert!(!mode_is_too_broad(0o400));
        assert!(mode_is_too_broad(0o640));
        assert!(mode_is_too_broad(0o644));
        assert!(mode_is_too_broad(0o602));
    }

    #[cfg(unix)]
    #[test]
    fn config_file_check_tolerates_missing_files() {
        warn_if_config_file_is_not_private(Path::new("/nonexistent/nodelite/server.toml"));
    }
}
