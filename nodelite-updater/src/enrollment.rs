//! Authenticate the admin-staged first executable before replacing the legacy unit.

use std::path::Path;

use crate::error::{Result, UpdateError};
use crate::policy::Policy;
use crate::state::{Artifact, Phase, Store};
use crate::{download, fs};

pub(crate) async fn verify(
    store: &Store,
    policy: &Policy,
    bootstrap: &[u8],
    baseline: &Path,
) -> Result<()> {
    if store
        .private()
        .join("recovery-required.json")
        .try_exists()?
    {
        return Err(UpdateError::Recovery("operator recovery required"));
    }
    if !store.private().join("journal.json").try_exists()? {
        store.initialize(policy, bootstrap, baseline)?;
    }
    let mut state = store.load(policy, bootstrap)?;
    if state.phase != Phase::Stable || state.transaction.is_some() {
        return Err(UpdateError::Recovery(
            "enrollment cannot reset a transaction",
        ));
    }
    let candidate = download::fetch(store, &mut state, policy, bootstrap).await?;
    matches_baseline(&candidate, policy)?;
    // Only the legacy group passes this command's guard: no live Agent executes these pointers.
    state.installed = candidate;
    store.save(&state)?;
    fs::pointer(&store.code(), "active", &state.installed.digest)?;
    fs::pointer(&store.code(), "candidate", &state.installed.digest)
}

fn matches_baseline(candidate: &Artifact, policy: &Policy) -> Result<()> {
    if candidate.digest != policy.baseline_sha256 || candidate.sequence != policy.baseline_sequence
    {
        return Err(UpdateError::Policy(
            "enrollment bundle is not the repository's authorized current baseline",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enrollment_requires_the_exact_signed_baseline_not_merely_a_newer_release() {
        let policy = crate::policy::fixture();
        let baseline = Artifact {
            digest: policy.baseline_sha256.clone(),
            sequence: policy.baseline_sequence,
        };
        assert!(matches_baseline(&baseline, &policy).is_ok());
        let newer = Artifact {
            sequence: baseline.sequence + 1,
            ..baseline.clone()
        };
        assert!(matches_baseline(&newer, &policy).is_err());
        let changed = Artifact {
            digest: crate::fs::digest(b"different"),
            ..baseline.clone()
        };
        assert!(matches_baseline(&changed, &policy).is_err());
    }
}
