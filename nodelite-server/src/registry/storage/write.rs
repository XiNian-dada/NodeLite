use std::fs::{File, OpenOptions};
use std::io::Write;
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::fs_security::{create_private_dir_all, ensure_directory_mode};

use super::super::validate::validate_registry_file;
use super::super::{RegistryError, RegistryFile, RegistryResult};
use super::{MAX_REGISTRY_WRITE_RETRIES, load_registry_file_sync};

#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

pub(super) fn save_registry_file_sync(path: &Path, file: &RegistryFile) -> RegistryResult<()> {
    validate_registry_file(path, file)?;
    ensure_registry_parent_dir(path)?;

    let payload = serde_json::to_string_pretty(file).map_err(RegistryError::serialize)?;
    let tmp_path = temporary_registry_path(path)?;
    write_registry_payload(&tmp_path, &payload)?;
    harden_registry_permissions(&tmp_path)?;
    std::fs::rename(&tmp_path, path)
        .map_err(|error| RegistryError::io("replacing", path, error))?;
    sync_parent_dir(path);
    verify_registry_permissions(path)?;
    Ok(())
}

pub(super) fn mutate_registry_file_sync<T, F>(
    path: &Path,
    operation: F,
) -> RegistryResult<(T, RegistryFile)>
where
    F: Fn(&mut RegistryFile) -> RegistryResult<(T, bool)> + Clone,
{
    ensure_registry_parent_dir(path)?;
    for _attempt in 0..MAX_REGISTRY_WRITE_RETRIES {
        let mut file = load_registry_file_sync(path)?;
        let base_version = file.version;
        let (value, should_persist) = operation(&mut file)?;
        if !should_persist {
            return Ok((value, file));
        }

        file.version = base_version.saturating_add(1);
        validate_registry_file(path, &file)?;

        let tmp_path = temporary_registry_path(path)?;
        let payload = serde_json::to_string_pretty(&file).map_err(RegistryError::serialize)?;
        write_registry_payload(&tmp_path, &payload)?;
        harden_registry_permissions(&tmp_path)?;

        match commit_prepared_registry_write(path, base_version, &tmp_path) {
            Ok(()) => return Ok((value, file)),
            Err(RegistryError::VersionConflict { .. }) => {
                cleanup_temporary_registry_file(&tmp_path);
                std::thread::sleep(Duration::from_millis(1));
            }
            Err(error) => {
                cleanup_temporary_registry_file(&tmp_path);
                return Err(error);
            }
        }
    }

    Err(RegistryError::validation(format!(
        "registry mutation exceeded {} optimistic write retries",
        MAX_REGISTRY_WRITE_RETRIES
    )))
}

fn ensure_registry_parent_dir(path: &Path) -> RegistryResult<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        create_private_dir_all(parent).map_err(|error| {
            RegistryError::internal("failed to create registry directory", error)
        })?;
    }
    Ok(())
}

fn commit_prepared_registry_write(
    path: &Path,
    expected_version: u64,
    tmp_path: &Path,
) -> RegistryResult<()> {
    let _lock = acquire_registry_lock(path)?;
    let current = load_registry_file_sync(path)?;
    if current.version != expected_version {
        return Err(RegistryError::version_conflict(
            expected_version,
            current.version,
        ));
    }
    std::fs::rename(tmp_path, path).map_err(|error| RegistryError::io("replacing", path, error))?;
    sync_parent_dir(path);
    verify_registry_permissions(path)?;
    Ok(())
}

fn temporary_registry_path(path: &Path) -> RegistryResult<PathBuf> {
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("server.json");
    // 并发写时固定 tmp 名会互相覆盖;加随机后缀让每个写操作拿到独立临时文件。
    let mut suffix = [0u8; 8];
    getrandom::fill(&mut suffix).map_err(|error| {
        RegistryError::internal(
            "failed to generate registry temp-file suffix",
            anyhow::anyhow!(error),
        )
    })?;
    let suffix_hex = suffix
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    Ok(path.with_file_name(format!("{file_name}.tmp.{suffix_hex}")))
}

fn cleanup_temporary_registry_file(path: &Path) {
    let _ = std::fs::remove_file(path);
}

fn write_registry_payload(path: &Path, payload: &str) -> RegistryResult<()> {
    let mut options = OpenOptions::new();
    options.create(true).truncate(true).write(true);
    #[cfg(unix)]
    options.mode(0o600);

    let mut file = options
        .open(path)
        .map_err(|error| RegistryError::io("opening", path, error))?;
    file.write_all(payload.as_bytes())
        .map_err(|error| RegistryError::io("writing", path, error))?;
    // rename 前确保数据已经刷盘,避免主机崩溃后留下空的注册表文件 —— 注册表丢失
    // 等于所有 Agent 鉴权失败,后果比一次写入失败更严重。
    file.sync_all()
        .map_err(|error| RegistryError::io("fsyncing", path, error))?;
    Ok(())
}

/// rename 之后 fsync 父目录,使新目录项随之持久化。
/// 打不开父目录(权限等)时静默退出 —— 数据已经 fsync,目录项丢失只意味着回退到上一份注册表。
fn sync_parent_dir(path: &Path) {
    let Some(parent) = path.parent() else {
        return;
    };
    if parent.as_os_str().is_empty() {
        return;
    }
    let _ = std::fs::File::open(parent).and_then(|dir| dir.sync_all());
}

fn registry_lock_path(path: &Path) -> PathBuf {
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("server.json");
    path.with_file_name(format!("{file_name}.lock"))
}

fn acquire_registry_lock(path: &Path) -> RegistryResult<RegistryFileLock> {
    let lock_path = registry_lock_path(path);
    if let Some(parent) = lock_path.parent()
        && !parent.as_os_str().is_empty()
    {
        create_private_dir_all(parent).map_err(|error| {
            RegistryError::internal("failed to create registry lock directory", error)
        })?;
    }

    let mut options = OpenOptions::new();
    options.create(true).read(true).write(true);
    #[cfg(unix)]
    options.mode(0o600);

    let file = options
        .open(&lock_path)
        .map_err(|error| RegistryError::io("opening", &lock_path, error))?;
    harden_registry_permissions(&lock_path)?;
    lock_file_exclusive(&file, &lock_path)?;
    Ok(RegistryFileLock { file, lock_path })
}

fn lock_file_exclusive(file: &File, lock_path: &Path) -> RegistryResult<()> {
    #[cfg(unix)]
    {
        let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) };
        if result != 0 {
            return Err(RegistryError::io(
                "locking",
                lock_path,
                std::io::Error::last_os_error(),
            ));
        }
    }

    #[cfg(not(unix))]
    {
        let _ = (file, lock_path);
    }

    Ok(())
}

fn unlock_file(file: &File) {
    #[cfg(unix)]
    {
        let _ = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_UN) };
    }

    #[cfg(not(unix))]
    {
        let _ = file;
    }
}

fn harden_registry_permissions(path: &Path) -> RegistryResult<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        ensure_directory_mode(parent, 0o700).map_err(|error| {
            RegistryError::internal("failed to harden registry parent directory", error)
        })?;
    }
    #[cfg(unix)]
    {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
            .map_err(|error| RegistryError::io("chmod-ing", path, error))?;
    }

    #[cfg(not(unix))]
    {
        let _ = path;
    }

    Ok(())
}

fn verify_registry_permissions(path: &Path) -> RegistryResult<()> {
    #[cfg(unix)]
    {
        let mode = std::fs::metadata(path)
            .map_err(|error| RegistryError::io("stat-ing", path, error))?
            .permissions()
            .mode()
            & 0o777;
        if mode != 0o600 {
            return Err(RegistryError::validation(format!(
                "{} must be mode 0600, got {mode:o}",
                path.display()
            )));
        }
    }

    #[cfg(not(unix))]
    {
        let _ = path;
    }

    Ok(())
}

struct RegistryFileLock {
    file: File,
    lock_path: PathBuf,
}

impl Drop for RegistryFileLock {
    fn drop(&mut self) {
        release_registry_lock_with(
            || unlock_file(&self.file),
            || {
                let _ = harden_registry_permissions(&self.lock_path);
            },
        );
    }
}

pub(in crate::registry) fn release_registry_lock_with<U, H>(unlock: U, harden: H)
where
    U: FnOnce(),
    H: FnOnce(),
{
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(unlock));
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(harden));
}
