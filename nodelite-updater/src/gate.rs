//! Decides when unattended reconcile may exit cleanly so it never holds back Agent startup.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Gate {
    /// Run the normal recovery path, which fails closed on anything unexpected.
    Proceed,
    /// Nothing may change, but failing here would block `nodelite-agent.service` via `Requires=`.
    Idle(&'static str),
}

/// Verified facts read under the lock after the journal and `active` object were checked.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Facts<'a> {
    pub paused: bool,
    pub recovery_required: bool,
    pub terminal: bool,
    pub active: &'a str,
    pub installed: &'a str,
    pub from: Option<&'a str>,
}

/// Idles only when `active` is already a known-good object; anything else keeps recovery or
/// fails closed, so a paused policy can never skip a rollback or start an unconfirmed candidate.
pub(crate) fn reconcile(facts: Facts<'_>) -> Gate {
    if facts.recovery_required {
        let known_good = facts.active == facts.installed || Some(facts.active) == facts.from;
        return if known_good {
            Gate::Idle("operator recovery required; recovery state preserved")
        } else {
            Gate::Proceed
        };
    }
    if facts.paused && facts.terminal && facts.active == facts.installed {
        return Gate::Idle("signed updates paused by local policy");
    }
    Gate::Proceed
}

#[cfg(test)]
mod tests {
    use super::*;

    const INSTALLED: &str = "installed";
    const FROM: &str = "from";
    const TO: &str = "to";

    fn facts(paused: bool, recovery: bool, terminal: bool, active: &'static str) -> Facts<'static> {
        Facts {
            paused,
            recovery_required: recovery,
            terminal,
            active,
            installed: INSTALLED,
            from: (!terminal).then_some(FROM),
        }
    }

    #[test]
    fn paused_policy_idles_only_a_settled_installed_object() {
        assert!(matches!(
            reconcile(facts(true, false, true, INSTALLED)),
            Gate::Idle(_)
        ));
        // Committed pointer changed: normal reconcile fails closed.
        assert_eq!(reconcile(facts(true, false, true, TO)), Gate::Proceed);
    }

    #[test]
    fn paused_policy_never_skips_an_open_transaction() {
        // Prepared, Stopping, Trial and RollbackIntent must still roll forward or back.
        for active in [FROM, TO, INSTALLED] {
            assert_eq!(reconcile(facts(true, false, false, active)), Gate::Proceed);
        }
    }

    #[test]
    fn enabled_policy_always_runs_recovery_without_latch() {
        for terminal in [true, false] {
            for active in [INSTALLED, FROM, TO] {
                assert_eq!(
                    reconcile(facts(false, false, terminal, active)),
                    Gate::Proceed
                );
            }
        }
    }

    #[test]
    fn recovery_latch_idles_only_on_a_verified_known_good_object() {
        for paused in [true, false] {
            assert!(matches!(
                reconcile(facts(paused, true, true, INSTALLED)),
                Gate::Idle(_)
            ));
            assert!(matches!(
                reconcile(facts(paused, true, false, FROM)),
                Gate::Idle(_)
            ));
            // An unconfirmed candidate or unexpected object keeps the Agent blocked.
            assert_eq!(reconcile(facts(paused, true, false, TO)), Gate::Proceed);
            assert_eq!(reconcile(facts(paused, true, true, TO)), Gate::Proceed);
        }
    }

    #[test]
    fn reconcile_timer_never_spawns_during_manual_recovery() {
        let unit = include_str!("../systemd/nodelite-updater-reconcile.service");
        assert!(unit.contains("ConditionPathExists=/var/lib/nodelite-updater/private/pending"));
        assert!(unit.contains(
            "ConditionPathExists=!/var/lib/nodelite-updater/private/recovery-required.json"
        ));
    }
}
