//! The monitor can request one fixed operation, but cannot write trust, code or updater state.

use nodelite_proto::agent_update::{
    AgentUpdateCapability as Capability, AgentUpdatePhase as Phase, AgentUpdateReason as Reason,
    AgentUpdateRequest, AgentUpdateStatus,
};

pub(crate) fn status() -> AgentUpdateStatus {
    #[cfg(target_os = "linux")]
    {
        linux::status()
            .unwrap_or_else(|reason| AgentUpdateStatus::unavailable(Capability::Disabled, reason))
    }
    #[cfg(not(target_os = "linux"))]
    AgentUpdateStatus::unavailable(Capability::Unsupported, Reason::UnsupportedEnvironment)
}

pub(crate) fn request(request: AgentUpdateRequest) -> AgentUpdateStatus {
    let mut status = status();
    let now = chrono::Utc::now().timestamp().max(0) as u64;
    let reason = if !request.validate(now) {
        Some(Reason::InvalidRequest)
    } else if status.capability != Capability::Ready {
        status.reason.or(Some(Reason::DisabledByPolicy))
    } else if status.phase.in_progress() {
        Some(Reason::Busy)
    } else if status.request_id.is_some()
        && now
            < status
                .updated_at
                .saturating_add(nodelite_proto::agent_update::UPDATE_COOLDOWN_SECS)
    {
        Some(Reason::RateLimited)
    } else {
        #[cfg(target_os = "linux")]
        {
            linux::enqueue(&request).err()
        }
        #[cfg(not(target_os = "linux"))]
        Some(Reason::UnsupportedEnvironment)
    };
    status.request_id = Some(request.request_id);
    status.updated_at = now;
    status.phase = if reason.is_some() {
        Phase::Failed
    } else {
        Phase::Queued
    };
    status.reason = reason;
    status
}

#[cfg(target_os = "linux")]
mod linux {
    use std::fs::{File, OpenOptions};
    use std::io::{Read, Write};
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
    use std::path::Path;

    use super::*;

    const HOME: &str = "/var/lib/nodelite-updater";
    const POLICY: &str = "/etc/nodelite-updater/policy.toml";

    fn read(path: &Path, limit: usize) -> Result<Vec<u8>, Reason> {
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(path)
            .map_err(|_| Reason::LocalStateInvalid)?;
        let info = file.metadata().map_err(|_| Reason::LocalStateInvalid)?;
        if !info.is_file() || info.nlink() != 1 || info.len() > limit as u64 {
            return Err(Reason::LocalStateInvalid);
        }
        let mut bytes = Vec::new();
        file.take(limit as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| Reason::LocalStateInvalid)?;
        if bytes.len() > limit {
            return Err(Reason::LocalStateInvalid);
        }
        Ok(bytes)
    }

    fn policy() -> Result<(), Reason> {
        let process = std::fs::read_to_string("/proc/self/status")
            .map_err(|_| Reason::UnsupportedEnvironment)?;
        // A writable executable must never inherit privileges when a local administrator changes a unit.
        // SAFETY: the effective UID query returns a scalar without caller-provided memory.
        if unsafe { libc::geteuid() } == 0
            || ["CapEff:", "CapPrm:", "CapAmb:"].iter().any(|name| {
                !process.lines().any(|line| {
                    line.strip_prefix(name)
                        .is_some_and(|value| value.trim() == "0000000000000000")
                })
            })
        {
            return Err(Reason::UnsupportedEnvironment);
        }
        if !Path::new(POLICY).exists() {
            return Err(Reason::NotEnrolled);
        }
        for path in Path::new(POLICY).ancestors() {
            let info = path
                .symlink_metadata()
                .map_err(|_| Reason::LocalStateInvalid)?;
            if info.file_type().is_symlink() || info.uid() != 0 || info.mode() & 0o022 != 0 {
                return Err(Reason::LocalStateInvalid);
            }
        }
        let data = read(Path::new(POLICY), 16384)?;
        let text = std::str::from_utf8(&data).map_err(|_| Reason::LocalStateInvalid)?;
        let document: toml_edit::DocumentMut =
            text.parse().map_err(|_| Reason::LocalStateInvalid)?;
        if document.get("enabled").and_then(toml_edit::Item::as_bool) != Some(true)
            || document
                .get("allow_remote_updates")
                .and_then(toml_edit::Item::as_bool)
                != Some(true)
        {
            return Err(Reason::DisabledByPolicy);
        }
        if !crate::update_health::enabled()
            || Path::new("/.dockerenv").exists()
            || Path::new("/run/systemd/container").exists()
            || !Path::new("/sys/fs/cgroup/cgroup.controllers").is_file()
        {
            return Err(Reason::UnsupportedEnvironment);
        }
        Ok(())
    }

    fn layout() -> Result<(), Reason> {
        // SAFETY: identity queries return scalars without caller-provided memory.
        let (uid, gid) = unsafe { (libc::geteuid(), libc::getegid()) };
        for path in ["/var", "/var/lib", HOME] {
            let info = Path::new(path)
                .symlink_metadata()
                .map_err(|_| Reason::LocalStateInvalid)?;
            if !info.is_dir() || info.uid() != 0 || info.mode() & 0o022 != 0 {
                return Err(Reason::LocalStateInvalid);
            }
        }
        for (name, mode) in [("requests", 0o770), ("public", 0o750)] {
            let info = Path::new(HOME)
                .join(name)
                .symlink_metadata()
                .map_err(|_| Reason::LocalStateInvalid)?;
            if !info.is_dir()
                || info.mode() & 0o7777 != mode
                || info.gid() != gid
                || (name == "requests" && info.uid() != 0)
                || (name == "public" && (info.uid() == uid || info.uid() == 0))
            {
                return Err(Reason::LocalStateInvalid);
            }
        }
        Ok(())
    }

    pub(super) fn status() -> Result<AgentUpdateStatus, Reason> {
        policy()?;
        layout()?;
        let path = Path::new(HOME).join("public/status.json");
        let owner = path
            .parent()
            .ok_or(Reason::LocalStateInvalid)?
            .symlink_metadata()
            .map_err(|_| Reason::LocalStateInvalid)?
            .uid();
        let info = path
            .symlink_metadata()
            .map_err(|_| Reason::LocalStateInvalid)?;
        if info.uid() != owner || info.mode() & 0o7777 != 0o640 {
            return Err(Reason::LocalStateInvalid);
        }
        let status: AgentUpdateStatus =
            serde_json::from_slice(&read(&path, 4096)?).map_err(|_| Reason::LocalStateInvalid)?;
        if !status.validate() {
            return Err(Reason::LocalStateInvalid);
        }
        Ok(status)
    }

    pub(super) fn enqueue(request: &AgentUpdateRequest) -> Result<(), Reason> {
        let directory = Path::new(HOME).join("requests");
        let destination = directory.join("request.json");
        if destination.symlink_metadata().is_ok() {
            return Err(Reason::Busy);
        }
        let temporary = directory.join(format!(".request-{}", request.request_id));
        let result = (|| -> std::io::Result<()> {
            let mut file = OpenOptions::new()
                .create_new(true)
                .write(true)
                .mode(0o640)
                .custom_flags(libc::O_NOFOLLOW)
                .open(&temporary)?;
            file.write_all(&serde_json::to_vec(request)?)?;
            file.sync_all()?;
            std::fs::rename(&temporary, &destination)?;
            File::open(&directory)?.sync_all()
        })();
        let _ = std::fs::remove_file(temporary);
        result.map_err(|_| Reason::LocalStateInvalid)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unenrolled_agent_never_executes_an_update_request() {
        let now = chrono::Utc::now().timestamp().max(0) as u64;
        let status = request(AgentUpdateRequest {
            request_id: "a".repeat(32),
            issued_at: now,
            expires_at: now + 60,
        });
        assert_eq!(status.phase, Phase::Failed);
        assert_ne!(status.capability, Capability::Ready);
        assert!(status.validate());
    }
}
