//! 注册表文件持久化与加载模块。
//!
//! 负责从磁盘 JSON 文件安全读取注册表、执行旧版本元数据与密码学迁移、校验体积上限并计算文件指纹。

mod write;

use std::fs::Metadata;
use std::path::Path;
use std::time::SystemTime;

use chrono::Utc;
use tokio::fs;

use self::write::mutate_registry_file_sync;
#[cfg(test)]
pub(super) use self::write::release_registry_lock_with;
use super::migration::migrate_legacy_display_metadata;
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

/// 磁盘文件必须满足的写入/加载前提。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RegistryGuard {
    /// 没有内存状态可保护：启动、CLI 或尚未有任何提交的空注册表。
    AllowCreate,
    /// 内存持有已提交状态：文件必须存在且版本不低于它。缺失单独判断，
    /// 因为无 `version` 字段的旧文件版本也是 0，版本比较区分不出删除。
    RequireExisting { min_version: u64 },
}

impl RegistryGuard {
    pub(super) fn check(self, path: &Path, disk_version: Option<u64>) -> RegistryResult<()> {
        let Self::RequireExisting { min_version } = self else {
            return Ok(());
        };
        match disk_version {
            Some(version) if version >= min_version => Ok(()),
            disk_version => Err(RegistryError::diverged(path, disk_version, min_version)),
        }
    }
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
    let state = load_registry_state(path, RegistryGuard::AllowCreate).await?;
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

/// 在任何迁移或清理回写之前先用原始磁盘版本校验 `guard`，
/// 否则回写会抬高回退文件的版本，使旧内容被当作新提交接受。
pub(super) async fn load_registry_state(
    path: &Path,
    guard: RegistryGuard,
) -> RegistryResult<RegistryState> {
    let existing = read_registry_file(path).await?;
    guard.check(path, existing.as_ref().map(|file| file.version))?;
    let mut file = existing.unwrap_or_default();
    // 过期安装会话仍带明文 session token，只在内存里丢弃会让它留在磁盘上直到下一次写入。
    let pruned_sessions = prune_expired_install_sessions(&mut file, Utc::now());

    // 展示字段先迁移到当前安全边界，再执行严格校验；通过后才做昂贵的
    // Argon2 token 迁移，避免损坏的注册表在拒绝前消耗额外资源。
    let metadata_migrated = migrate_legacy_display_metadata(&mut file);
    validate_registry_file(path, &file)?;
    let legacy_tokens = file
        .nodes
        .iter()
        .any(|node| node.token_hash.is_empty() && !node.token.is_empty());
    if metadata_migrated || legacy_tokens || pruned_sessions {
        file = migrate_registry_file(path, guard).await?;
    }

    load_registry_state_from_file(path, file)
}

pub(super) async fn migrate_registry_file(
    path: &Path,
    guard: RegistryGuard,
) -> RegistryResult<RegistryFile> {
    // Rebase migrations on the latest durable file using the same commit protection as edits.
    let ((), file) = mutate_registry_file(path, guard, |file| {
        prune_expired_install_sessions(file, Utc::now());
        migrate_legacy_tokens(file)?;
        // The synchronous loader has already normalized legacy display metadata.
        Ok(((), true))
    })
    .await?;
    Ok(file)
}

async fn read_registry_file(path: &Path) -> RegistryResult<Option<RegistryFile>> {
    let metadata = match fs::metadata(path).await {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(None);
        }
        Err(error) => return Err(RegistryError::io("stat-ing", path, error)),
    };
    ensure_registry_file_size(path, metadata.len())?;

    let content = match fs::read_to_string(path).await {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(None);
        }
        Err(error) => return Err(RegistryError::io("reading", path, error)),
    };
    #[cfg(test)]
    record_registry_file_read(path);

    let file: RegistryFile =
        serde_json::from_str(&content).map_err(|error| RegistryError::parse(path, error))?;
    Ok(Some(file))
}

#[cfg(test)]
pub(super) fn load_registry_file_sync(path: &Path) -> RegistryResult<RegistryFile> {
    Ok(read_registry_file_sync(path)?.unwrap_or_default())
}

pub(super) fn read_registry_file_sync(path: &Path) -> RegistryResult<Option<RegistryFile>> {
    let metadata = match std::fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(None);
        }
        Err(error) => return Err(RegistryError::io("stat-ing", path, error)),
    };
    ensure_registry_file_size(path, metadata.len())?;

    let content = match std::fs::read_to_string(path) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(None);
        }
        Err(error) => return Err(RegistryError::io("reading", path, error)),
    };

    let mut file: RegistryFile =
        serde_json::from_str(&content).map_err(|error| RegistryError::parse(path, error))?;
    migrate_legacy_display_metadata(&mut file);
    validate_registry_file(path, &file)?;
    Ok(Some(file))
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
        version: file.version,
        entries,
        install_sessions,
    })
}

/// 在 `spawn_blocking` 中以"无锁准备 → 版本校验 → 原子替换"的方式更新注册表文件。
///
/// 重的 JSON 解析 / 序列化 / tmp 文件写入都发生在锁外; 独占 flock 只覆盖
/// "确认 registry 版本未变化 + rename 提交" 这一步。若发现版本冲突就丢弃
/// 当前准备结果并基于最新文件重试。
///
/// `guard` 描述调用方内存已提交的状态；不满足时拒绝写入，
/// 否则会以回退后的文件为基线覆盖掉内存里仍在服务的节点。
pub(super) async fn mutate_registry_file<T, F>(
    path: &Path,
    guard: RegistryGuard,
    operation: F,
) -> RegistryResult<(T, RegistryFile)>
where
    T: Send + 'static,
    F: Fn(&mut RegistryFile) -> RegistryResult<(T, bool)> + Send + Clone + 'static,
{
    let path = path.to_path_buf();
    tokio::task::spawn_blocking(move || mutate_registry_file_sync(&path, guard, operation))
        .await
        .map_err(RegistryError::mutation_task)?
}
