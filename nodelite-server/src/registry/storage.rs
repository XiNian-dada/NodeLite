mod write;

use std::fs::Metadata;
use std::path::Path;
use std::time::SystemTime;

use chrono::Utc;
use tokio::fs;

#[cfg(test)]
pub(super) use self::write::release_registry_lock_with;
use self::write::{mutate_registry_file_sync, save_registry_file_sync};
use super::token::{migrate_legacy_tokens, prune_expired_install_sessions};
use super::validate::validate_registry_file;
use super::{RegistryError, RegistryFile, RegistryResult, RegistryState};

#[cfg(unix)]
use std::os::unix::fs::MetadataExt;

const MAX_REGISTRY_WRITE_RETRIES: usize = 32;
pub(super) const MAX_REGISTRY_FILE_BYTES: u64 = 4 * 1024 * 1024;

#[cfg(test)]
type RegistryFileReadCounts = std::collections::HashMap<std::path::PathBuf, u64>;

#[cfg(test)]
static REGISTRY_FILE_READS: std::sync::OnceLock<std::sync::Mutex<RegistryFileReadCounts>> =
    std::sync::OnceLock::new();

#[cfg(test)]
fn registry_file_reads() -> &'static std::sync::Mutex<RegistryFileReadCounts> {
    REGISTRY_FILE_READS.get_or_init(|| std::sync::Mutex::new(RegistryFileReadCounts::new()))
}

#[cfg(test)]
pub(super) fn reset_registry_file_read_count(path: &Path) {
    registry_file_reads()
        .lock()
        .expect("registry read count mutex should not be poisoned")
        .remove(path);
}

#[cfg(test)]
pub(super) fn registry_file_read_count(path: &Path) -> u64 {
    registry_file_reads()
        .lock()
        .expect("registry read count mutex should not be poisoned")
        .get(path)
        .copied()
        .unwrap_or(0)
}

#[cfg(test)]
fn record_registry_file_read(path: &Path) {
    let mut counts = registry_file_reads()
        .lock()
        .expect("registry read count mutex should not be poisoned");
    *counts.entry(path.to_path_buf()).or_default() += 1;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RegistryFileFingerprint {
    Missing,
    Present {
        len: u64,
        modified: Option<SystemTime>,
        #[cfg(unix)]
        dev: u64,
        #[cfg(unix)]
        ino: u64,
    },
}

pub(super) async fn registry_file_fingerprint(
    path: &Path,
) -> RegistryResult<RegistryFileFingerprint> {
    match fs::metadata(path).await {
        Ok(metadata) => Ok(RegistryFileFingerprint::from_metadata(&metadata)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Ok(RegistryFileFingerprint::Missing)
        }
        Err(error) => Err(RegistryError::io("stat-ing", path, error)),
    }
}

pub(super) async fn load_registry_state_with_fingerprint(
    path: &Path,
) -> RegistryResult<(RegistryState, RegistryFileFingerprint)> {
    let fingerprint = registry_file_fingerprint(path).await?;
    let state = load_registry_state(path).await?;
    Ok((state, fingerprint))
}

impl RegistryFileFingerprint {
    fn from_metadata(metadata: &Metadata) -> Self {
        Self::Present {
            len: metadata.len(),
            modified: metadata.modified().ok(),
            #[cfg(unix)]
            dev: metadata.dev(),
            #[cfg(unix)]
            ino: metadata.ino(),
        }
    }
}

pub(super) async fn load_registry_state(path: &Path) -> RegistryResult<RegistryState> {
    let mut file = load_registry_file(path).await?;
    prune_expired_install_sessions(&mut file, Utc::now());

    // #56: 升级老版本的明文 token 到 Argon2id 哈希。一旦发现旧字段, 哈希后
    // 立即落盘, 之后磁盘上不再有任何节点的明文。
    let migrated = migrate_legacy_tokens(&mut file)?;
    if migrated {
        file.version = file.version.saturating_add(1);
        let path_buf = path.to_path_buf();
        let file_clone = file.clone();
        tokio::task::spawn_blocking(move || save_registry_file_sync(&path_buf, &file_clone))
            .await
            .map_err(RegistryError::background_task)??;
    }

    load_registry_state_from_file(path, file)
}

async fn load_registry_file(path: &Path) -> RegistryResult<RegistryFile> {
    let metadata = match fs::metadata(path).await {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(RegistryFile::default());
        }
        Err(error) => return Err(RegistryError::io("stat-ing", path, error)),
    };
    ensure_registry_file_size(path, metadata.len())?;

    let content = match fs::read_to_string(path).await {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(RegistryFile::default());
        }
        Err(error) => return Err(RegistryError::io("reading", path, error)),
    };
    #[cfg(test)]
    record_registry_file_read(path);

    let file: RegistryFile =
        serde_json::from_str(&content).map_err(|error| RegistryError::parse(path, error))?;
    validate_registry_file(path, &file)?;
    Ok(file)
}

pub(super) fn load_registry_file_sync(path: &Path) -> RegistryResult<RegistryFile> {
    let metadata = match std::fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(RegistryFile::default());
        }
        Err(error) => return Err(RegistryError::io("stat-ing", path, error)),
    };
    ensure_registry_file_size(path, metadata.len())?;

    let content = match std::fs::read_to_string(path) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(RegistryFile::default());
        }
        Err(error) => return Err(RegistryError::io("reading", path, error)),
    };

    let file: RegistryFile =
        serde_json::from_str(&content).map_err(|error| RegistryError::parse(path, error))?;
    validate_registry_file(path, &file)?;
    Ok(file)
}

fn ensure_registry_file_size(path: &Path, len: u64) -> RegistryResult<()> {
    if len > MAX_REGISTRY_FILE_BYTES {
        return Err(RegistryError::file_too_large(
            path,
            len,
            MAX_REGISTRY_FILE_BYTES,
        ));
    }
    Ok(())
}

pub(super) fn load_registry_state_from_file(
    path: &Path,
    file: RegistryFile,
) -> RegistryResult<RegistryState> {
    let mut entries = std::collections::HashMap::with_capacity(file.nodes.len());
    for node in file.nodes {
        if entries.insert(node.node_id.clone(), node).is_some() {
            return Err(RegistryError::validation(format!(
                "duplicate node_id found in {}",
                path.display()
            )));
        }
    }
    let mut install_sessions =
        std::collections::HashMap::with_capacity(file.install_sessions.len());
    for session in file.install_sessions {
        if install_sessions
            .insert(session.token.clone(), session)
            .is_some()
        {
            return Err(RegistryError::validation(format!(
                "duplicate install token found in {}",
                path.display()
            )));
        }
    }

    Ok(RegistryState {
        entries,
        install_sessions,
    })
}

/// 在 `spawn_blocking` 中以"无锁准备 → 版本校验 → 原子替换"的方式更新注册表文件。
///
/// 重的 JSON 解析 / 序列化 / tmp 文件写入都发生在锁外; 独占 flock 只覆盖
/// "确认 registry 版本未变化 + rename 提交" 这一步。若发现版本冲突就丢弃
/// 当前准备结果并基于最新文件重试。
pub(super) async fn mutate_registry_file<T, F>(
    path: &Path,
    operation: F,
) -> RegistryResult<(T, RegistryFile)>
where
    T: Send + 'static,
    F: Fn(&mut RegistryFile) -> RegistryResult<(T, bool)> + Send + Clone + 'static,
{
    let path = path.to_path_buf();
    tokio::task::spawn_blocking(move || mutate_registry_file_sync(&path, operation))
        .await
        .map_err(RegistryError::mutation_task)?
}
