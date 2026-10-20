//! Durable publication and bounded inode snapshots without following input links.

use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::error::{Result, UpdateError};

pub(crate) const PAYLOAD_LIMIT: usize = 32 * 1024 * 1024;

pub(crate) fn digest(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

pub(crate) fn nonce() -> Result<String> {
    let mut bytes = [0; 16];
    getrandom::fill(&mut bytes).map_err(|_| UpdateError::Randomness)?;
    Ok(hex::encode(bytes))
}

pub(crate) fn read(path: &Path, limit: usize) -> Result<Vec<u8>> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)?;
    let info = file.metadata()?;
    if !info.is_file() || info.nlink() != 1 || info.len() > limit as u64 {
        return Err(UpdateError::Recovery("unsafe or oversized input inode"));
    }
    let mut bytes = Vec::new();
    file.take(limit as u64 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > limit {
        return Err(UpdateError::Recovery("input grew beyond its limit"));
    }
    Ok(bytes)
}

pub(crate) fn sync_dir(path: &Path) -> Result<()> {
    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW)
        .open(path)?
        .sync_all()?;
    Ok(())
}

pub(crate) fn directory(path: &Path, mode: u32) -> Result<()> {
    std::fs::create_dir(path)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))?;
    Ok(())
}

struct Temporary(PathBuf);
impl Drop for Temporary {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

pub(crate) fn atomic(path: &Path, bytes: &[u8], mode: u32) -> Result<()> {
    let parent = path
        .parent()
        .ok_or(UpdateError::Recovery("missing parent"))?;
    let temporary = Temporary(parent.join(format!(".update-{}", nonce()?)));
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .custom_flags(libc::O_NOFOLLOW)
        .mode(mode)
        .open(&temporary.0)?;
    file.set_permissions(std::fs::Permissions::from_mode(mode))?;
    file.write_all(bytes)?;
    file.sync_all()?;
    std::fs::rename(&temporary.0, path)?;
    sync_dir(parent)
}

pub(crate) fn json<T: serde::Serialize>(path: &Path, item: &T) -> Result<()> {
    atomic(path, &serde_json::to_vec(item)?, 0o600)
}

pub(crate) fn pointer(code: &Path, name: &str, hash: &str) -> Result<()> {
    crate::policy::sanitize_digest(hash)?;
    let temporary = Temporary(code.join(format!(".pointer-{}", nonce()?)));
    std::os::unix::fs::symlink(format!("objects/{hash}"), &temporary.0)?;
    std::fs::rename(&temporary.0, code.join(name))?;
    sync_dir(code)
}

pub(crate) struct Lock(File);
impl Lock {
    pub(crate) fn acquire(path: &Path) -> Result<Self> {
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(path)?;
        let info = file.metadata()?;
        if !info.is_file() || info.nlink() != 1 {
            return Err(UpdateError::Recovery("unsafe transaction lock"));
        }
        // SAFETY: file owns a live FD for the call and its lifetime retains the advisory lock.
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            let error = std::io::Error::last_os_error();
            return Err(if error.kind() == std::io::ErrorKind::WouldBlock {
                UpdateError::Busy
            } else {
                error.into()
            });
        }
        Ok(Self(file))
    }
}
impl Drop for Lock {
    fn drop(&mut self) {
        // SAFETY: the owned file is still open here; unlock precedes the descriptor's drop.
        unsafe {
            libc::flock(self.0.as_raw_fd(), libc::LOCK_UN);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atomic_publication_has_exact_mode_and_no_aliased_inode() {
        let dir = tempfile::tempdir().expect("temporary directory");
        let path = dir.path().join("state");
        atomic(&path, b"first", 0o600).expect("first publication");
        let held = File::open(&path).expect("old inode");
        atomic(&path, b"second", 0o600).expect("second publication");
        assert_ne!(
            held.metadata().expect("metadata").ino(),
            std::fs::metadata(&path).expect("metadata").ino()
        );
        assert_eq!(
            std::fs::metadata(&path).expect("metadata").mode() & 0o777,
            0o600
        );
        assert_eq!(read(&path, 32).expect("read"), b"second");
    }

    #[test]
    fn links_and_overlong_inputs_are_rejected() {
        let dir = tempfile::tempdir().expect("temporary directory");
        let original = dir.path().join("original");
        std::fs::write(&original, b"bytes").expect("write");
        assert!(read(&original, 1).is_err());
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(&original, &link).expect("symlink");
        assert!(read(&link, 10).is_err());
        std::fs::remove_file(&link).expect("remove symlink");
        std::fs::hard_link(&original, &link).expect("hardlink");
        assert!(read(&link, 10).is_err());
    }

    #[test]
    fn transaction_lock_is_exclusive() {
        let dir = tempfile::tempdir().expect("temporary directory");
        let path = dir.path().join("lock");
        let _first = Lock::acquire(&path).expect("first lock");
        assert!(matches!(Lock::acquire(&path), Err(UpdateError::Busy)));
    }
}
