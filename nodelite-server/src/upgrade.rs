//! 生成升级与回滚所需文件清单（Manifest），无需连接数据库且杜绝凭据泄漏。

use std::collections::BTreeSet;
use std::io::{self, Write};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::path::{Path, PathBuf};

use thiserror::Error;

use nodelite_proto::{ServerConfig, parse_server_config};

use crate::passkeys::passkey_storage_path;

#[derive(Debug, Error)]
pub enum UpgradeError {
    #[error("failed to prepare upgrade manifest: {0}")]
    Io(#[from] io::Error),
    #[error("server configuration cannot be parsed for upgrade preparation")]
    Config,
    #[error("upgrade paths must be UTF-8 without line breaks or NUL bytes")]
    UnsupportedPath,
}

pub(crate) async fn print_upgrade_manifest(config_path: &Path) -> Result<(), UpgradeError> {
    let content = tokio::fs::read_to_string(config_path).await?;
    // TOML 解析错误片段可能包含明文密码，直接透传会导致凭据泄漏到更新日志中。
    let config = parse_server_config(&content).map_err(|_| UpgradeError::Config)?;
    let manifest = render_manifest(config_path, &config, &std::env::current_dir()?)?;
    io::stdout().lock().write_all(manifest.as_bytes())?;
    Ok(())
}

fn render_manifest(
    config_path: &Path,
    config: &ServerConfig,
    working_dir: &Path,
) -> Result<String, UpgradeError> {
    let mut paths = BTreeSet::new();
    let passkey_path = passkey_storage_path(config_path);
    for path in [
        config_path,
        passkey_path.as_path(),
        config.node_registry_path.as_path(),
        config.snapshot_path.as_path(),
        config.geoip.database_path.as_path(),
    ] {
        // 原子写入机制可能会替换软链接本身而非直接修改其目标，因此需要同时备份软链接和规范化真实路径。
        paths.extend(backup_paths(path, working_dir)?);
    }
    for database in [&config.history_db_path, &config.audit.db_path] {
        let [configured, database] = backup_paths(database, working_dir)?;
        paths.insert(configured);
        // 仅停止服务无法保证之前的意外崩溃已执行干净的 checkpoint，需备份所有 WAL/SHM/Journal 辅助文件。
        for suffix in ["", "-wal", "-shm", "-journal"] {
            let mut path = database.as_os_str().to_os_string();
            path.push(suffix);
            paths.insert(PathBuf::from(path));
        }
    }
    let mut manifest = format!(
        "nodelite-upgrade-manifest-v1\nversion={}\nready_url={}\n",
        crate::server_build_version(),
        readiness_url(config.listen),
    );
    for path in paths {
        manifest.push_str("path=");
        manifest.push_str(path.to_str().ok_or(UpgradeError::UnsupportedPath)?);
        manifest.push('\n');
    }
    Ok(manifest)
}

fn backup_paths(path: &Path, working_dir: &Path) -> Result<[PathBuf; 2], UpgradeError> {
    let absolute = working_dir.join(path);
    let resolved = match absolute.symlink_metadata() {
        Ok(_) => absolute.canonicalize()?,
        Err(error) if error.kind() == io::ErrorKind::NotFound => absolute.clone(),
        Err(error) => return Err(error.into()),
    };
    for path in [&absolute, &resolved] {
        let text = path.to_str().ok_or(UpgradeError::UnsupportedPath)?;
        if text.chars().any(|ch| matches!(ch, '\n' | '\r' | '\0')) {
            return Err(UpgradeError::UnsupportedPath);
        }
    }
    Ok([absolute, resolved])
}

fn readiness_url(mut listen: SocketAddr) -> String {
    if listen.ip().is_unspecified() {
        listen.set_ip(match listen.ip() {
            IpAddr::V4(_) => IpAddr::V4(Ipv4Addr::LOCALHOST),
            IpAddr::V6(_) => IpAddr::V6(Ipv6Addr::LOCALHOST),
        });
    }
    format!("http://{listen}/readyz")
}

#[cfg(test)]
mod tests;
