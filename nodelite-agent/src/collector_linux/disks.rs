//! Linux mount filtering and statvfs capacity reads, with injectable syscalls for tests.

use std::collections::HashSet;
use std::ffi::CString;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result, anyhow};
use tracing::warn;

use nodelite_proto::{DiskUsage, percentage};

/// `statvfs` 探测函数的签名。生产环境指向真正的 libc 系统调用,
/// 测试时可注入桩实现,从而避免对宿主机真实根文件系统的依赖。
pub(super) type StatvfsFn = fn(&str) -> Result<FilesystemStats>;

/// 遍历 `/proc/mounts` 并通过 `statvfs` 获取各挂载点的容量信息。
/// 同一挂载点重复出现时只保留第一条;特殊虚拟文件系统会被忽略。
///
/// `statvfs_fn` 由调用方注入,生产环境是真实系统调用,测试时为桩实现。
pub(super) fn collect_disks(
    mounts_path: &std::path::Path,
    statvfs_fn: StatvfsFn,
) -> Result<Vec<DiskUsage>> {
    let content = fs::read_to_string(mounts_path)
        .with_context(|| format!("read {}", mounts_path.display()))?;
    let mut seen_mounts = HashSet::new();
    let mut seen_devices = HashSet::new();
    let mut disks = Vec::new();

    for line in content.lines() {
        let mut fields = line.split_whitespace();
        let Some(raw_device) = fields.next() else {
            continue;
        };
        let Some(raw_mount_point) = fields.next() else {
            continue;
        };
        let Some(raw_fs_type) = fields.next() else {
            continue;
        };
        let device = unescape_mount_field(raw_device);
        let mount_point = unescape_mount_field(raw_mount_point);
        let fs_type = raw_fs_type.to_string();

        if ignored_filesystems().contains(&fs_type.as_str())
            || !seen_mounts.insert(mount_point.clone())
        {
            continue;
        }

        let stats = match statvfs_fn(&mount_point) {
            Ok(stats) => stats,
            Err(error) => {
                warn!(
                    mount_point = %mount_point,
                    fs_type = %fs_type,
                    error = ?error,
                    "skipping disk mount after statvfs failure",
                );
                continue;
            }
        };
        if stats.total_bytes == 0 {
            continue;
        }
        let device_identity = format!("{device}:{}", stats.total_bytes);
        if !seen_devices.insert(device_identity) {
            continue;
        }

        disks.push(DiskUsage {
            device,
            mount_point,
            fs_type,
            total_bytes: stats.total_bytes,
            available_bytes: stats.available_bytes,
            used_bytes: stats.used_bytes,
            used_percent: percentage(stats.used_bytes, stats.total_bytes),
        });
    }

    disks.sort_by(|left, right| left.mount_point.cmp(&right.mount_point));
    Ok(disks)
}

/// 默认忽略的"非物理"文件系统,这些通常代表内核虚拟视图或临时挂载。
fn ignored_filesystems() -> &'static [&'static str] {
    &[
        "autofs",
        "bpf",
        "cgroup",
        "cgroup2",
        "configfs",
        "debugfs",
        "devpts",
        "devtmpfs",
        "fusectl",
        "mqueue",
        "overlay",
        "proc",
        "pstore",
        "ramfs",
        "securityfs",
        "squashfs",
        "sysfs",
        "tmpfs",
        "tracefs",
    ]
}

/// `/proc/mounts` 中的空格会被转义为 `\040`,这里还原回真实字符。
fn unescape_mount_field(value: &str) -> String {
    value.replace("\\040", " ")
}

pub(super) struct FilesystemStats {
    pub(super) total_bytes: u64,
    pub(super) available_bytes: u64,
    pub(super) used_bytes: u64,
}

/// 调用 libc 的 `statvfs` 获取挂载点容量,以字节为单位返回。
/// 这是 [`StatvfsFn`] 的生产实现;测试通过 [`super::HostCollector::with_statvfs`] 注入桩替换它。
pub(super) fn real_statvfs(path: &str) -> Result<FilesystemStats> {
    let c_path =
        CString::new(path.as_bytes()).with_context(|| format!("path contains NUL byte: {path}"))?;
    let mut stats = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    // SAFETY: `c_path` is a live NUL-terminated C string and `stats` points to
    // writable storage large enough for libc to fill one `statvfs` value.
    let result = unsafe { libc::statvfs(c_path.as_ptr(), stats.as_mut_ptr()) };
    if result != 0 {
        return Err(anyhow!("statvfs failed for {}", Path::new(path).display()));
    }
    // SAFETY: `statvfs` returned success, which means libc initialized `stats`.
    let stats = unsafe { stats.assume_init() };

    let block_size = stats.f_frsize;
    let total_blocks = stats.f_blocks;
    let available_blocks = stats.f_bavail;
    let total_bytes = total_blocks.saturating_mul(block_size);
    let available_bytes = available_blocks.saturating_mul(block_size);
    let used_bytes = total_bytes.saturating_sub(available_bytes);

    Ok(FilesystemStats {
        total_bytes,
        available_bytes,
        used_bytes,
    })
}
