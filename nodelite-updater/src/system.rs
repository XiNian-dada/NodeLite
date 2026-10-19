//! Only root-owned fixed units can act on the Agent; the updater writes fixed markers.

use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

use nodelite_proto::update_health::UpdateHealth;
use tokio::process::Command;

use crate::error::{Result, UpdateError};
use crate::fs;
use crate::state::{HOME, Store, epoch};

pub(crate) const AGENT: &str = "nodelite-agent.service";
pub(crate) const PREFLIGHT: &str = "nodelite-agent-preflight.service";
pub(crate) const STOP: &str = "nodelite-updater-stop.service";
const BRIDGE: &str = "/var/lib/nodelite-updater/bridge";

pub(crate) async fn properties(unit: &str) -> Result<BTreeMap<String, String>> {
    if !matches!(unit, AGENT | PREFLIGHT | STOP) {
        return Err(UpdateError::Policy("non-fixed service rejected"));
    }
    let output = tokio::time::timeout(Duration::from_secs(3), Command::new("/usr/bin/systemctl")
        .args(["show", unit, "--property=MainPID,InvocationID,ActiveState,ControlGroup,Result,ExecMainExitTimestampMonotonic,User,Group,AmbientCapabilities,CapabilityBoundingSet"])
        .kill_on_drop(true).output()).await.map_err(|_| UpdateError::Timeout)??;
    if !output.status.success() || output.stdout.len() > 16384 {
        return Err(UpdateError::Policy("cannot inspect fixed service"));
    }
    let text = std::str::from_utf8(&output.stdout)
        .map_err(|_| UpdateError::Policy("invalid systemd properties"))?;
    Ok(text
        .lines()
        .filter_map(|line| line.split_once('=').map(|(a, b)| (a.into(), b.into())))
        .collect())
}

pub(crate) fn value<'a>(properties: &'a BTreeMap<String, String>, name: &str) -> &'a str {
    properties.get(name).map(String::as_str).unwrap_or("")
}

pub(crate) async fn guard() -> Result<()> {
    if !Path::new("/sys/fs/cgroup/cgroup.controllers").is_file()
        || Path::new("/.dockerenv").exists()
        || Path::new("/run/systemd/container").exists()
    {
        return Err(UpdateError::Policy(
            "requires non-container Linux systemd with cgroup v2",
        ));
    }
    let status = std::fs::read_to_string("/proc/self/status")?;
    let capabilities = status.lines().find_map(|l| l.strip_prefix("CapEff:\t"));
    let output = Command::new("/usr/bin/id").arg("-un").output().await?;
    let group = Command::new("/usr/bin/id").arg("-gn").output().await?;
    if capabilities != Some("0000000000000000")
        || output.stdout != b"nodelite-updater\n"
        || group.stdout != b"nodelite-read\n"
    {
        return Err(UpdateError::Policy(
            "requires dedicated capability-free updater UID",
        ));
    }
    let service = properties(AGENT).await?;
    if value(&service, "User") != "nodelite-agent"
        || value(&service, "Group") != "nodelite-read"
        || !value(&service, "AmbientCapabilities").is_empty()
        || !value(&service, "CapabilityBoundingSet").is_empty()
    {
        return Err(UpdateError::Policy(
            "unsupported Agent unit or privileged traffic control",
        ));
    }
    Ok(())
}

pub(crate) fn boot_id() -> Result<String> {
    Ok(std::fs::read_to_string("/proc/sys/kernel/random/boot_id")?
        .trim()
        .into())
}

pub(crate) fn boottime_ms() -> Result<u64> {
    let mut time = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // Absolute boot time is comparable between updater invocations, unlike Instant values.
    if unsafe { libc::clock_gettime(libc::CLOCK_BOOTTIME, &mut time) } != 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(time.tv_sec as u64 * 1000 + time.tv_nsec as u64 / 1_000_000)
}

pub(crate) fn marker(action: &str) -> Result<()> {
    if !matches!(action, "restart" | "stop" | "preflight") {
        return Err(UpdateError::Policy("unknown fixed action"));
    }
    fs::atomic(&Path::new(BRIDGE).join(action), b"request\n", 0o600)
}

pub(crate) fn clear_markers() -> Result<()> {
    for name in ["restart", "stop", "preflight"] {
        let path = Path::new(BRIDGE).join(name);
        match std::fs::remove_file(path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
    }
    fs::sync_dir(Path::new(BRIDGE))
}

pub(crate) fn marker_pending(name: &str) -> bool {
    Path::new(BRIDGE).join(name).symlink_metadata().is_ok()
}

pub(crate) fn stopped(properties: &BTreeMap<String, String>) -> Result<bool> {
    if value(properties, "ActiveState") != "inactive" || value(properties, "MainPID") != "0" {
        return Ok(false);
    }
    let cgroup = value(properties, "ControlGroup");
    if cgroup.is_empty() {
        return Ok(true);
    }
    if !cgroup.starts_with("/system.slice/") || cgroup.split('/').any(|part| part == "..") {
        return Err(UpdateError::Recovery("unexpected Agent cgroup"));
    }
    let path = Path::new("/sys/fs/cgroup")
        .join(cgroup.trim_start_matches('/'))
        .join("cgroup.procs");
    match std::fs::read_to_string(path) {
        Ok(processes) => Ok(processes.trim().is_empty()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(true),
        Err(e) => Err(e.into()),
    }
}

pub(crate) fn health(
    store: &Store,
    process: &BTreeMap<String, String>,
    expected: &str,
) -> Result<Option<UpdateHealth>> {
    if value(process, "ActiveState") != "active" || store.active()? != expected {
        return Ok(None);
    }
    let evidence = fs::read(Path::new("/run/nodelite-agent/update-health.json"), 4096)
        .and_then(|bytes| Ok(serde_json::from_slice::<UpdateHealth>(&bytes)?));
    let Ok(evidence) = evidence else {
        return Ok(None);
    };
    let now = boottime_ms()?;
    let wall = epoch()? * 1000;
    if !evidence.validate()
        || evidence.executable_sha256 != expected
        || evidence.invocation != value(process, "InvocationID")
        || evidence.pid.to_string() != value(process, "MainPID")
        || evidence.sample_boottime_ms > now
        || now - evidence.sample_boottime_ms > 15000
        || evidence.collected_at_ms > wall + 1000
        || wall.saturating_sub(evidence.collected_at_ms) > 15000
    {
        return Ok(None);
    }
    Ok(Some(evidence))
}

pub(crate) fn manual(store: &Store, reason: &'static str) -> Result<()> {
    fs::json(
        &store.private().join("recovery-required.json"),
        &serde_json::json!({"reason": reason}),
    )
}

pub(crate) fn paths_safe(store: &Store) -> Result<()> {
    use std::os::unix::fs::MetadataExt;
    // Disk ownership must match the kernel identity, not an environment variable.
    let uid = unsafe { libc::geteuid() };
    let gid = unsafe { libc::getegid() };
    for path in [
        Path::new("/var"),
        Path::new("/var/lib"),
        Path::new(HOME),
        &store.private(),
        &store.code(),
        &store.code().join("objects"),
    ] {
        let metadata = std::fs::symlink_metadata(path)?;
        let private = path == store.private();
        let mutable = private || path.starts_with(store.code());
        let expected_mode = if private {
            0o700
        } else if mutable {
            0o750
        } else {
            0o755
        };
        if !metadata.is_dir()
            || metadata.mode() & 0o777 != expected_mode
            || metadata.uid() != if mutable { uid } else { 0 }
            || mutable && !private && metadata.gid() != gid
        {
            return Err(UpdateError::Recovery("unsafe updater directory"));
        }
    }
    for name in ["generations", "roots"] {
        let path = store.private().join(name);
        if let Ok(metadata) = path.symlink_metadata()
            && (!metadata.is_dir() || metadata.uid() != uid || metadata.mode() & 0o777 != 0o700)
        {
            return Err(UpdateError::Recovery("unsafe trust cache directory"));
        }
    }
    let bridge = Path::new(BRIDGE).symlink_metadata()?;
    if !bridge.is_dir()
        || bridge.uid() != 0
        || bridge.gid() != store.private().metadata()?.gid()
        || bridge.mode() & 0o777 != 0o770
    {
        return Err(UpdateError::Recovery("unsafe fixed-action bridge"));
    }
    Ok(())
}
