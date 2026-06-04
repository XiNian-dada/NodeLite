//! Server-update subprocess probing and launch strategy.
//!
//! `start_server_update` needs one place that can be tested without invoking the
//! real systemd binary. This module isolates the `systemd-run --version` probe,
//! classifies timeout/non-zero/missing outcomes, and fails closed when the
//! sandboxed launcher is unavailable.

use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command as StdCommand, Stdio};
use std::time::Duration;

use tokio::process::Command;
use tokio::time::timeout;
use tracing::warn;

const SYSTEMD_PROBE_TIMEOUT: Duration = Duration::from_secs(60);
#[cfg(test)]
const TEST_LAUNCH_WAIT_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum UpdateLaunchMode {
    Systemd,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SystemdAvailability {
    Available,
    Missing,
    TimedOut,
    Failed,
}

struct UpdateLauncher {
    systemd_run: PathBuf,
    probe_timeout: Duration,
}

impl Default for UpdateLauncher {
    fn default() -> Self {
        Self {
            systemd_run: PathBuf::from("systemd-run"),
            probe_timeout: SYSTEMD_PROBE_TIMEOUT,
        }
    }
}

pub(super) async fn spawn_server_update_subprocess(
    unit_name: &str,
    command: &str,
    writable_paths: &[PathBuf],
) -> io::Result<UpdateLaunchMode> {
    UpdateLauncher::default()
        .spawn_server_update(unit_name, command, writable_paths)
        .await
}

#[cfg(test)]
async fn is_systemd_available_at(systemd_run: &Path, probe_timeout: Duration) -> bool {
    matches!(
        probe_systemd_availability(systemd_run, probe_timeout).await,
        SystemdAvailability::Available
    )
}

async fn probe_systemd_availability(
    systemd_run: &Path,
    probe_timeout: Duration,
) -> SystemdAvailability {
    probe_command_availability(systemd_run, &["--version"], probe_timeout).await
}

async fn probe_command_availability(
    program: &Path,
    args: &[&str],
    probe_timeout: Duration,
) -> SystemdAvailability {
    let mut child = match Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
    {
        Ok(child) => child,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return SystemdAvailability::Missing;
        }
        Err(_) => return SystemdAvailability::Failed,
    };

    match timeout(probe_timeout, child.wait()).await {
        Ok(Ok(status)) if status.success() => SystemdAvailability::Available,
        Ok(Ok(_)) => SystemdAvailability::Failed,
        Ok(Err(error)) if error.kind() == io::ErrorKind::NotFound => SystemdAvailability::Missing,
        Ok(Err(_)) => SystemdAvailability::Failed,
        Err(_) => {
            let _ = child.start_kill();
            let _ = child.wait().await;
            SystemdAvailability::TimedOut
        }
    }
}

impl UpdateLauncher {
    async fn spawn_server_update(
        &self,
        unit_name: &str,
        command: &str,
        writable_paths: &[PathBuf],
    ) -> io::Result<UpdateLaunchMode> {
        let availability = if self.systemd_run == Path::new("systemd-run")
            && self.probe_timeout == SYSTEMD_PROBE_TIMEOUT
        {
            probe_systemd_availability(Path::new("systemd-run"), SYSTEMD_PROBE_TIMEOUT).await
        } else {
            probe_systemd_availability(&self.systemd_run, self.probe_timeout).await
        };
        self.spawn_server_update_with_probe(unit_name, command, writable_paths, availability)
    }

    fn spawn_server_update_with_probe(
        &self,
        unit_name: &str,
        command: &str,
        writable_paths: &[PathBuf],
        availability: SystemdAvailability,
    ) -> io::Result<UpdateLaunchMode> {
        match availability {
            SystemdAvailability::Available => {
                self.spawn_systemd_run(unit_name, command, writable_paths)?;
                Ok(UpdateLaunchMode::Systemd)
            }
            SystemdAvailability::Missing => {
                warn!(
                    "systemd-run is unavailable for server updates; refusing unsafe shell fallback"
                );
                Err(unsupported_update_launcher())
            }
            SystemdAvailability::TimedOut => {
                warn!(
                    "systemd-run probe timed out for server updates; refusing unsafe shell fallback"
                );
                Err(unsupported_update_launcher())
            }
            SystemdAvailability::Failed => {
                warn!(
                    "systemd-run probe failed for server updates; refusing unsafe shell fallback"
                );
                Err(unsupported_update_launcher())
            }
        }
    }

    fn spawn_systemd_run(
        &self,
        unit_name: &str,
        command: &str,
        writable_paths: &[PathBuf],
    ) -> io::Result<()> {
        let mut systemd_run = StdCommand::new(&self.systemd_run);
        systemd_run
            .arg(format!("--unit={unit_name}"))
            .arg("--collect")
            .arg("--service-type=exec")
            .arg("--property=ProtectSystem=full")
            .arg("--property=ProtectHome=yes")
            .arg("--property=PrivateTmp=yes")
            .arg("--property=NoNewPrivileges=yes");
        for path in writable_paths {
            systemd_run.arg(format!("--property=ReadWritePaths={}", path.display()));
        }
        systemd_run
            .arg("sh")
            .arg("-c")
            .arg(command)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map(|_| ())
    }
}

fn unsupported_update_launcher() -> io::Error {
    io::Error::new(
        io::ErrorKind::Unsupported,
        "manual server update requires systemd-run sandboxing",
    )
}

#[cfg(test)]
mod tests {
    use std::env;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    use tokio::time::{Duration, sleep};

    use crate::encoding::shell_quote;

    use super::{
        SYSTEMD_PROBE_TIMEOUT, SystemdAvailability, TEST_LAUNCH_WAIT_TIMEOUT, UpdateLaunchMode,
        UpdateLauncher, is_systemd_available_at, probe_command_availability,
        probe_systemd_availability,
    };

    struct TempScriptDir {
        path: PathBuf,
    }

    impl TempScriptDir {
        fn new(label: &str) -> Self {
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock should be monotonic enough")
                .as_nanos();
            let path = env::temp_dir().join(format!("nodelite-{label}-{unique}"));
            fs::create_dir_all(&path).expect("temp dir should exist");
            Self { path }
        }

        fn write_script(&self, name: &str, body: &str) -> PathBuf {
            let path = self.path.join(name);
            fs::write(&path, body).expect("script should be written");
            let mut permissions = fs::metadata(&path)
                .expect("script metadata should exist")
                .permissions();
            permissions.set_mode(0o700);
            fs::set_permissions(&path, permissions).expect("script should be executable");
            path
        }
    }

    impl Drop for TempScriptDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    #[test]
    fn temp_script_dir_uses_system_temp_root() {
        let dir = TempScriptDir::new("temp-root");

        assert!(dir.path.starts_with(env::temp_dir()));
    }

    #[tokio::test]
    async fn is_systemd_available_reports_successful_probe() {
        let command = Path::new("/usr/bin/true");

        assert!(is_systemd_available_at(command, Duration::from_millis(200)).await);
        assert_eq!(
            probe_systemd_availability(command, SYSTEMD_PROBE_TIMEOUT).await,
            SystemdAvailability::Available,
        );
    }

    #[tokio::test]
    async fn is_systemd_available_reports_missing_binary() {
        let missing = PathBuf::from("/definitely/not/present/systemd-run");

        assert!(!is_systemd_available_at(&missing, Duration::from_millis(50)).await);
        assert_eq!(
            probe_systemd_availability(&missing, Duration::from_millis(50)).await,
            SystemdAvailability::Missing,
        );
    }

    #[tokio::test]
    async fn is_systemd_available_reports_timeout_probe() {
        assert_eq!(
            probe_command_availability(Path::new("/bin/sleep"), &["1"], Duration::from_millis(20),)
                .await,
            SystemdAvailability::TimedOut,
        );
    }

    #[tokio::test]
    async fn is_systemd_available_reports_non_zero_probe() {
        assert_eq!(
            probe_command_availability(
                Path::new("/bin/sh"),
                &["-c", "exit 7"],
                Duration::from_millis(200),
            )
            .await,
            SystemdAvailability::Failed,
        );
    }

    #[tokio::test]
    async fn spawn_server_update_uses_systemd_when_available() {
        let dir = TempScriptDir::new("systemd-launcher");
        let marker = dir.path.join("systemd.args");
        let systemd_script = dir.write_script(
            "systemd-run",
            &format!(
                "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then exit 0; fi\nprintf '%s\\n' \"$@\" > {}\n",
                shell_quote(&marker.display().to_string())
            ),
        );
        let launcher = UpdateLauncher {
            systemd_run: systemd_script,
            probe_timeout: Duration::from_millis(50),
        };

        let mode = launcher
            .spawn_server_update_with_probe(
                "nodelite-test-unit",
                "echo update",
                &[
                    PathBuf::from("/opt/nodelite"),
                    PathBuf::from("/usr/local/bin"),
                ],
                SystemdAvailability::Available,
            )
            .expect("systemd launch should succeed");

        assert_eq!(mode, UpdateLaunchMode::Systemd);
        let captured = wait_for_file(&marker).await;
        assert!(captured.contains("--unit=nodelite-test-unit"));
        assert!(captured.contains("--property=ReadWritePaths=/opt/nodelite"));
        assert!(captured.contains("--property=ReadWritePaths=/usr/local/bin"));
    }

    #[tokio::test]
    async fn spawn_server_update_rejects_missing_systemd_without_shell_fallback() {
        let dir = TempScriptDir::new("systemd-fallback");
        let marker = dir.path.join("shell.args");
        let launcher = UpdateLauncher {
            systemd_run: PathBuf::from("/missing/systemd-run"),
            probe_timeout: Duration::from_millis(50),
        };

        let error = launcher
            .spawn_server_update_with_probe(
                "ignored-unit",
                "echo fallback",
                &[],
                SystemdAvailability::Missing,
            )
            .expect_err("missing systemd should fail closed");

        assert_eq!(error.kind(), std::io::ErrorKind::Unsupported);
        sleep(Duration::from_millis(50)).await;
        assert!(
            !marker.exists(),
            "shell fallback should not have been spawned"
        );
    }

    async fn wait_for_file(path: &Path) -> String {
        let deadline = tokio::time::Instant::now() + TEST_LAUNCH_WAIT_TIMEOUT;
        while tokio::time::Instant::now() < deadline {
            if let Ok(text) = tokio::fs::read_to_string(path).await {
                return text;
            }
            sleep(Duration::from_millis(10)).await;
        }
        panic!("timed out waiting for {}", path.display());
    }
}
