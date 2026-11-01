//! Decides which idle states must exit cleanly so they never hold back Agent startup.

/// Commands that run unattended from systemd units at boot or on a timer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Unattended {
    Reconcile,
    Status,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Gate {
    Proceed,
    /// Nothing may change, but failing here would block `nodelite-agent.service` via `Requires=`.
    Idle(&'static str),
}

/// Only unattended commands are gated; operator-invoked commands keep failing loudly.
pub(crate) fn unattended(
    command: Option<Unattended>,
    policy_enabled: bool,
    recovery_required: bool,
) -> Gate {
    match command {
        None => Gate::Proceed,
        Some(_) if !policy_enabled => Gate::Idle("signed updates paused by local policy"),
        // Status must still report the recovery state to the operator.
        Some(Unattended::Reconcile) if recovery_required => {
            Gate::Idle("operator recovery required; recovery state preserved")
        }
        Some(_) => Gate::Proceed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_policy_idles_unattended_commands_only() {
        for command in [Unattended::Reconcile, Unattended::Status] {
            assert!(matches!(
                unattended(Some(command), false, false),
                Gate::Idle(_)
            ));
            assert!(matches!(
                unattended(Some(command), false, true),
                Gate::Idle(_)
            ));
        }
        assert_eq!(unattended(None, false, false), Gate::Proceed);
        assert_eq!(unattended(None, false, true), Gate::Proceed);
    }

    #[test]
    fn manual_recovery_idles_reconcile_but_not_status_or_operator_commands() {
        assert!(matches!(
            unattended(Some(Unattended::Reconcile), true, true),
            Gate::Idle(_)
        ));
        assert_eq!(
            unattended(Some(Unattended::Status), true, true),
            Gate::Proceed
        );
        assert_eq!(unattended(None, true, true), Gate::Proceed);
        assert_eq!(
            unattended(Some(Unattended::Reconcile), true, false),
            Gate::Proceed
        );
    }
}
