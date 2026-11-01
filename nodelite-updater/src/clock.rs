//! Operator recovery for the local time floor after a forward wall-clock jump.

use crate::error::UpdateError;

/// Shared with the download check so a no-op reset never leaves fetch blocked.
pub(crate) const SKEW_SECS: u64 = 5;

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum FloorReset {
    Unchanged,
    Lowered { old: u64, new: u64 },
}

/// Returns only the new time floor so sequence, digest and rollback floors cannot be lowered here.
pub(crate) fn plan(
    confirmed: bool,
    terminal: bool,
    recovery_required: bool,
    time_high: u64,
    now: u64,
) -> Result<FloorReset, UpdateError> {
    if !confirmed {
        return Err(UpdateError::Policy(
            "pass --confirm-clock-correct after verifying the system clock",
        ));
    }
    // An open transaction or recovery latch keeps its evidence untouched for the operator.
    if recovery_required {
        return Err(UpdateError::Recovery("operator recovery required"));
    }
    if !terminal {
        return Err(UpdateError::Busy);
    }
    if time_high <= now.saturating_add(SKEW_SECS) {
        return Ok(FloorReset::Unchanged);
    }
    Ok(FloorReset::Lowered {
        old: time_high,
        new: now,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refuses_without_confirmation_recovery_or_open_transaction() {
        assert!(matches!(
            plan(false, true, false, 9_000, 1_000),
            Err(UpdateError::Policy(_))
        ));
        assert!(matches!(
            plan(true, true, true, 9_000, 1_000),
            Err(UpdateError::Recovery(_))
        ));
        assert!(matches!(
            plan(true, false, false, 9_000, 1_000),
            Err(UpdateError::Busy)
        ));
    }

    #[test]
    fn floor_within_skew_is_left_unchanged() {
        assert_eq!(
            plan(true, true, false, 1_005, 1_000).ok(),
            Some(FloorReset::Unchanged)
        );
        assert_eq!(
            plan(true, true, false, 10, 1_000).ok(),
            Some(FloorReset::Unchanged)
        );
    }

    #[test]
    fn future_floor_is_lowered_to_now() {
        assert_eq!(
            plan(true, true, false, 1_006, 1_000).ok(),
            Some(FloorReset::Lowered {
                old: 1_006,
                new: 1_000
            })
        );
    }
}
