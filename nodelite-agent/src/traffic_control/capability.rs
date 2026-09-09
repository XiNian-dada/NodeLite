//! Explicit opt-in and effective privileges determine whether shaping is available.

use nodelite_proto::TrafficControlUnavailableReason;

pub(super) fn unavailable_reason() -> Option<TrafficControlUnavailableReason> {
    #[cfg(not(target_os = "linux"))]
    {
        Some(TrafficControlUnavailableReason::UnsupportedPlatform)
    }
    #[cfg(target_os = "linux")]
    {
        if std::env::var("NODELITE_AGENT_TRAFFIC_CONTROL").as_deref() != Ok("1") {
            return Some(TrafficControlUnavailableReason::Disabled);
        }
        let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
        if !has_net_admin(&status) {
            return Some(TrafficControlUnavailableReason::MissingCapability);
        }
        let found = std::env::var_os("PATH").is_some_and(|path| {
            use std::os::unix::fs::PermissionsExt;
            std::env::split_paths(&path).any(|directory| {
                std::fs::metadata(directory.join("tc")).is_ok_and(|metadata| {
                    metadata.is_file() && metadata.permissions().mode() & 0o111 != 0
                })
            })
        });
        (!found).then_some(TrafficControlUnavailableReason::MissingTc)
    }
}

#[cfg(any(target_os = "linux", test))]
pub(super) fn has_net_admin(status: &str) -> bool {
    status
        .lines()
        .find_map(|line| line.strip_prefix("CapEff:"))
        .and_then(|mask| u64::from_str_radix(mask.trim(), 16).ok())
        .is_some_and(|mask| mask & (1 << 12) != 0)
}

#[cfg(test)]
mod tests {
    #[test]
    fn requires_effective_net_admin_not_just_permitted_capability() {
        assert!(super::has_net_admin("CapEff:\t0000000000001000\n"));
        assert!(!super::has_net_admin(
            "CapPrm:\t0000000000001000\nCapEff:\t0\n"
        ));
        assert!(!super::has_net_admin("CapEff: invalid"));
    }
}
