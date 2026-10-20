//! Evidence is opt-in and published only after durable configuration load and authenticated sampling.

#[derive(Debug, thiserror::Error)]
#[error(
    "signed updates require agent.report_interval_secs between 1 and 10 seconds; use manual upgrades for longer intervals"
)]
pub(crate) struct ReportIntervalError;

pub(crate) fn enabled() -> bool {
    cfg!(target_os = "linux") && std::env::var("NODELITE_UPDATE_HEALTH").as_deref() == Ok("1")
}

pub(crate) fn validate_report_interval(seconds: u64) -> Result<(), ReportIntervalError> {
    // Leave time for scheduling jitter inside the unchanged 15-second freshness bound.
    if !(1..=10).contains(&seconds) {
        return Err(ReportIntervalError);
    }
    Ok(())
}

#[cfg(target_os = "linux")]
mod linux {
    use std::fs::{File, OpenOptions};
    use std::io::{Read, Write};
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    use std::path::{Path, PathBuf};
    use std::sync::OnceLock;
    use std::sync::atomic::{AtomicU64, Ordering};

    use nodelite_proto::update_health::UpdateHealth;
    use sha2::{Digest, Sha256};

    const STATUS: &str = "/run/nodelite-agent/update-health.json";
    static PUBLISHER: OnceLock<Option<Publisher>> = OnceLock::new();

    struct Publisher {
        digest: String,
        invocation: String,
        counter: AtomicU64,
    }

    fn initialize() -> Option<Publisher> {
        if std::env::var("NODELITE_UPDATE_HEALTH").as_deref() != Ok("1") {
            return None;
        }
        let invocation = std::env::var("INVOCATION_ID").ok()?;
        let mut file = File::open("/proc/self/exe").ok()?;
        let mut hash = Sha256::new();
        let mut buffer = [0; 65536];
        loop {
            let size = file.read(&mut buffer).ok()?;
            if size == 0 {
                break;
            }
            hash.update(&buffer[..size]);
        }
        let digest = format!("{:x}", hash.finalize());
        let health = UpdateHealth {
            schema: 1,
            pid: std::process::id(),
            invocation: invocation.clone(),
            executable_sha256: digest.clone(),
            sample_boottime_ms: 1,
            collected_at_ms: 1,
            sample_counter: 1,
        };
        health.validate().then_some(Publisher {
            digest,
            invocation,
            counter: AtomicU64::new(0),
        })
    }

    fn boottime_ms() -> std::io::Result<u64> {
        let mut value = libc::timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        // CLOCK_BOOTTIME is shared across these local UIDs and includes suspended time.
        if unsafe { libc::clock_gettime(libc::CLOCK_BOOTTIME, &mut value) } != 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok((value.tv_sec as u64) * 1000 + value.tv_nsec as u64 / 1_000_000)
    }

    struct Temporary(PathBuf);
    impl Drop for Temporary {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    fn write(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
        let mut suffix = [0; 8];
        getrandom::fill(&mut suffix).map_err(std::io::Error::other)?;
        let suffix: String = suffix.iter().map(|b| format!("{b:02x}")).collect();
        let temporary = Temporary(path.with_extension(format!("tmp.{suffix}")));
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o640)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&temporary.0)?;
        file.set_permissions(std::fs::Permissions::from_mode(0o640))?;
        file.write_all(bytes)?;
        std::fs::rename(&temporary.0, path)?;
        Ok(())
    }

    pub(super) fn publish(collected_at_ms: u64) {
        let Some(publisher) = PUBLISHER.get_or_init(initialize) else {
            return;
        };
        let result = (|| -> std::io::Result<()> {
            let health = UpdateHealth {
                schema: 1,
                pid: std::process::id(),
                invocation: publisher.invocation.clone(),
                executable_sha256: publisher.digest.clone(),
                sample_boottime_ms: boottime_ms()?,
                collected_at_ms,
                sample_counter: publisher
                    .counter
                    .fetch_add(1, Ordering::Relaxed)
                    .saturating_add(1),
            };
            write(Path::new(STATUS), &serde_json::to_vec(&health)?)
        })();
        if result.is_err() {
            tracing::warn!("could not publish local update health evidence");
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn health_clock_advances_and_is_not_wall_clock() {
            let first = boottime_ms().expect("boottime");
            assert!(first > 0);
            assert!(boottime_ms().expect("boottime") >= first);
        }
    }
}

pub(crate) async fn publish(collected_at_ms: u64) {
    if !enabled() {
        return;
    }
    #[cfg(target_os = "linux")]
    if let Err(error) = tokio::task::spawn_blocking(move || linux::publish(collected_at_ms)).await {
        tracing::warn!(%error, "local update health task failed");
    }
    #[cfg(not(target_os = "linux"))]
    let _ = collected_at_ms;
}

#[cfg(test)]
mod tests {
    #[test]
    fn signed_update_interval_leaves_a_freshness_margin() {
        for seconds in [1, 5, 10] {
            assert!(super::validate_report_interval(seconds).is_ok());
        }
        for seconds in [0, 11, 15, 20, 30, u64::MAX] {
            assert!(super::validate_report_interval(seconds).is_err());
        }
    }
}
