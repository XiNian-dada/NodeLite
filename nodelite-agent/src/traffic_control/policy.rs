//! Pure decisions about when tc must run and which interfaces hold NodeLite rules.
//!
//! The confirmed state is keyed by interface: the kernel rules live on a device,
//! so a rate confirmed on one default-route interface says nothing about another.

use std::collections::BTreeSet;

/// Policy that `tc` confirmed on a specific interface.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct AppliedPolicy {
    pub(super) rate_kbps: Option<u64>,
    pub(super) interface: String,
}

/// Work needed to make the kernel match the desired policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct TcPlan {
    pub(super) interface: String,
    pub(super) rate_kbps: Option<u64>,
    /// Interfaces where this process installed rules that no longer belong there.
    pub(super) stale_interfaces: Vec<String>,
}

/// Returns `None` when the confirmed state already matches rate and interface.
pub(super) fn plan(
    applied: Option<&AppliedPolicy>,
    owned: &BTreeSet<String>,
    rate_kbps: Option<u64>,
    interface: String,
) -> Option<TcPlan> {
    if applied
        .is_some_and(|applied| applied.rate_kbps == rate_kbps && applied.interface == interface)
    {
        return None;
    }
    let stale_interfaces = owned
        .iter()
        .filter(|owned| **owned != interface)
        .cloned()
        .collect();
    Some(TcPlan {
        interface,
        rate_kbps,
        stale_interfaces,
    })
}

/// Records which interfaces may hold NodeLite rules after an attempt.
pub(super) fn record_outcome(owned: &mut BTreeSet<String>, plan: &TcPlan, succeeded: bool) {
    if succeeded {
        owned.clear();
        if plan.rate_kbps.is_some() {
            owned.insert(plan.interface.clone());
        }
    } else {
        // Why: a partial failure may have left rules on the target; stale entries stay
        // tracked until a later attempt confirms their removal.
        owned.insert(plan.interface.clone());
    }
}

/// An active limit must follow the default route; a cleared limit has nothing to follow.
pub(super) fn route_drifted(applied: Option<&AppliedPolicy>, current: Option<&str>) -> bool {
    applied.is_some_and(|applied| {
        applied.rate_kbps.is_some() && current.is_some_and(|current| current != applied.interface)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn applied(rate: Option<u64>, interface: &str) -> AppliedPolicy {
        AppliedPolicy {
            rate_kbps: rate,
            interface: interface.to_string(),
        }
    }

    fn owned(names: &[&str]) -> BTreeSet<String> {
        names.iter().map(|name| (*name).to_string()).collect()
    }

    #[test]
    fn same_rate_on_same_interface_is_a_cache_hit() {
        let current = applied(Some(1000), "eth0");
        assert_eq!(
            plan(Some(&current), &owned(&["eth0"]), Some(1000), "eth0".into()),
            None
        );
    }

    #[test]
    fn same_rate_on_new_interface_reapplies_and_cleans_old_one() {
        let current = applied(Some(1000), "eth0");
        let plan = plan(Some(&current), &owned(&["eth0"]), Some(1000), "eth1".into())
            .expect("interface change must reapply");
        assert_eq!(plan.interface, "eth1");
        assert_eq!(plan.rate_kbps, Some(1000));
        assert_eq!(plan.stale_interfaces, vec!["eth0".to_string()]);
    }

    #[test]
    fn clearing_removes_rules_from_every_owned_interface() {
        let current = applied(Some(1000), "eth0");
        let plan = plan(
            Some(&current),
            &owned(&["eth0", "wg0"]),
            None,
            "eth1".into(),
        )
        .expect("clear must run");
        assert_eq!(plan.rate_kbps, None);
        assert_eq!(
            plan.stale_interfaces,
            vec!["eth0".to_string(), "wg0".to_string()]
        );
    }

    #[test]
    fn ownership_tracks_only_confirmed_or_possibly_partial_rules() {
        let mut tracked = owned(&["eth0"]);
        let move_plan = TcPlan {
            interface: "eth1".into(),
            rate_kbps: Some(1000),
            stale_interfaces: vec!["eth0".into()],
        };
        record_outcome(&mut tracked, &move_plan, false);
        assert_eq!(tracked, owned(&["eth0", "eth1"]));
        record_outcome(&mut tracked, &move_plan, true);
        assert_eq!(tracked, owned(&["eth1"]));
        let clear_plan = TcPlan {
            rate_kbps: None,
            ..move_plan
        };
        record_outcome(&mut tracked, &clear_plan, true);
        assert!(tracked.is_empty());
    }

    #[test]
    fn only_an_active_limit_follows_the_default_route() {
        let limited = applied(Some(1000), "eth0");
        assert!(route_drifted(Some(&limited), Some("eth1")));
        assert!(!route_drifted(Some(&limited), Some("eth0")));
        assert!(!route_drifted(Some(&limited), None));
        assert!(!route_drifted(Some(&applied(None, "eth0")), Some("eth1")));
        assert!(!route_drifted(None, Some("eth1")));
    }
}
