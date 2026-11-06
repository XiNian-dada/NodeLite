//! Web 更新锁探测与跨进程冲突错误分类。

use std::fs::OpenOptions;
use std::io;
use std::path::Path;

pub(super) fn check_update_idle(log: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::fd::AsRawFd;
        use std::os::unix::fs::OpenOptionsExt;
        let lock = OpenOptions::new()
            .create(true)
            .append(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open({
                let mut name = log.as_os_str().to_os_string();
                name.push(".lock");
                std::path::PathBuf::from(name)
            })?;
        // SAFETY: lock owns this valid fd; flock takes no pointers and the fd stays live.
        if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            return Err(io::Error::last_os_error());
        }
        drop(lock);
        Ok(())
    }
    #[cfg(not(unix))]
    {
        let _ = log;
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "server updates require Unix",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::fd::AsRawFd;
    #[test]
    fn active_script_lock_rejects_a_second_request_without_touching_log() {
        let path =
            std::env::temp_dir().join(format!("nodelite-update-lock-{}", std::process::id()));
        std::fs::write(&path, b"first update log").expect("fixture");
        let lock_path = {
            let mut name = path.as_os_str().to_os_string();
            name.push(".lock");
            std::path::PathBuf::from(name)
        };
        let lock = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&lock_path)
            .expect("lock file");
        assert_eq!(unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX) }, 0);
        assert_eq!(
            check_update_idle(&path).expect_err("busy").kind(),
            io::ErrorKind::WouldBlock
        );
        assert_eq!(std::fs::read(&path).expect("log"), b"first update log");
        drop(lock);
        check_update_idle(&path).expect("available after exit");
        std::fs::remove_file(path).expect("remove log");
        std::fs::remove_file(lock_path).expect("remove lock");
    }
}
