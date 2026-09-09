//! Describe rollback inputs without opening databases or serializing credentials.

use std::collections::BTreeSet;
use std::io::{self, Write};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::path::{Path, PathBuf};

use thiserror::Error;

use nodelite_proto::{ServerConfig, parse_server_config};

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
    // TOML error excerpts can contain passwords and would otherwise enter the update log.
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
    for path in [
        config_path,
        config.node_registry_path.as_path(),
        config.snapshot_path.as_path(),
        config.geoip.database_path.as_path(),
    ] {
        // Atomic persistence can replace a configured symlink instead of modifying its target.
        paths.extend(backup_paths(path, working_dir)?);
    }
    for database in [&config.history_db_path, &config.audit.db_path] {
        let [configured, database] = backup_paths(database, working_dir)?;
        paths.insert(configured);
        // Stopping the service does not guarantee a clean checkpoint after an earlier crash.
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
