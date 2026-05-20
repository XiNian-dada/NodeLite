// 文件系统权限辅助:
// - 统一把 server 运行期创建的目录收紧到 0700;
// - 在启动时记录权限过宽的目录,帮助运维发现潜在泄露面。

use std::path::Path;

use anyhow::{Context, Result};
use tracing::error;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

/// 确保目录存在并在 unix 上强制收敛到 0700。
pub(crate) fn create_private_dir_all(path: &Path) -> Result<()> {
    std::fs::create_dir_all(path)
        .with_context(|| format!("failed to create directory {}", path.display()))?;
    ensure_directory_mode(path, 0o700)?;
    Ok(())
}

/// 在 async 代码里复用的目录创建版本。
pub(crate) async fn create_private_dir_all_async(path: &Path) -> Result<()> {
    tokio::fs::create_dir_all(path)
        .await
        .with_context(|| format!("failed to create directory {}", path.display()))?;
    ensure_directory_mode(path, 0o700)?;
    Ok(())
}

/// 显式设置目录权限,避免依赖进程 umask。
pub(crate) fn ensure_directory_mode(path: &Path, mode: u32) -> Result<()> {
    #[cfg(unix)]
    {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
            .with_context(|| format!("failed to chmod {}", path.display()))?;
    }

    #[cfg(not(unix))]
    {
        let _ = (path, mode);
    }

    Ok(())
}

/// 启动时记录权限过宽的目录,但不阻断服务。
pub(crate) fn log_if_directory_is_not_private(path: &Path, field_name: &str) {
    #[cfg(unix)]
    {
        match std::fs::metadata(path) {
            Ok(metadata) => {
                let mode = metadata.permissions().mode() & 0o777;
                if mode & 0o077 != 0 {
                    error!(
                        field = field_name,
                        path = %path.display(),
                        mode = format!("{mode:o}"),
                        "directory permissions are broader than recommended 0700",
                    );
                }
            }
            Err(error) => {
                error!(
                    field = field_name,
                    path = %path.display(),
                    error = ?error,
                    "failed to inspect directory permissions",
                );
            }
        }
    }

    #[cfg(not(unix))]
    {
        let _ = (path, field_name);
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::{
        create_private_dir_all, create_private_dir_all_async, ensure_directory_mode,
        log_if_directory_is_not_private,
    };

    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    fn unique_temp_dir(prefix: &str) -> std::path::PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be monotonic enough")
            .as_nanos();
        std::env::temp_dir().join(format!("{prefix}-{unique}"))
    }

    #[cfg(unix)]
    fn assert_mode(path: &Path, expected_mode: u32) {
        let mode = std::fs::metadata(path)
            .expect("metadata should be readable")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, expected_mode);
    }

    #[test]
    fn create_private_dir_all_creates_directory_with_private_mode() {
        let path = unique_temp_dir("nodelite-fs-private-dir");
        create_private_dir_all(&path).expect("directory should be created");
        assert!(path.is_dir());
        #[cfg(unix)]
        assert_mode(&path, 0o700);
        let _ = std::fs::remove_dir_all(&path);
    }

    #[tokio::test]
    async fn create_private_dir_all_async_sets_private_mode() {
        let path = unique_temp_dir("nodelite-fs-private-dir-async");
        create_private_dir_all_async(&path)
            .await
            .expect("directory should be created");
        assert!(path.is_dir());
        #[cfg(unix)]
        assert_mode(&path, 0o700);
        let _ = std::fs::remove_dir_all(&path);
    }

    #[test]
    fn ensure_directory_mode_restricts_existing_directory() {
        let path = unique_temp_dir("nodelite-fs-mode-dir");
        std::fs::create_dir_all(&path).expect("directory should be created");
        #[cfg(unix)]
        {
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
                .expect("permissions should be widened");
        }
        ensure_directory_mode(&path, 0o700).expect("chmod should succeed");
        #[cfg(unix)]
        assert_mode(&path, 0o700);
        let _ = std::fs::remove_dir_all(&path);
    }

    #[test]
    fn log_if_directory_is_not_private_tolerates_missing_paths() {
        let path = unique_temp_dir("nodelite-fs-missing-dir");
        log_if_directory_is_not_private(&path, "missing_test_path");
    }
}
