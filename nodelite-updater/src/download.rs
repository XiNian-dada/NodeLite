//! Reference TUF verification feeds the durable trust journal before code activation.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use futures::TryStreamExt;
use serde::Deserialize;
use tough::{Limits, Repository, RepositoryLoader, TargetName};

use crate::error::{Result, UpdateError};
use crate::fs::{self, PAYLOAD_LIMIT};
use crate::policy::{Policy, sanitize_digest, sanitize_sequence};
use crate::state::{Artifact, Journal, Store, epoch, inventory};
use crate::transport::{Http, check_transport};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Release {
    product: String,
    component: String,
    channel: String,
    target: String,
    release_sequence: u64,
    min_rollback_sequence: u64,
    version: String,
}

impl Release {
    fn validate(&self, policy: &Policy) -> Result<()> {
        if self.product != "nodelite"
            || self.component != "agent"
            || self.channel != policy.channel
            || self.target != policy.target
            || self.version.is_empty()
            || self.version.len() > 64
            || !self
                .version
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b".-_".contains(&c))
        {
            return Err(UpdateError::Policy("signed release scope rejected"));
        }
        sanitize_sequence(self.release_sequence)?;
        sanitize_sequence(self.min_rollback_sequence)?;
        if self.min_rollback_sequence > self.release_sequence {
            return Err(UpdateError::Policy("signed rollback floor exceeds release"));
        }
        Ok(())
    }
}

fn stage(store: &Store, state: &Journal) -> Result<PathBuf> {
    let working = store.new_generation()?;
    if let Some(identity) = &state.cache {
        let previous = store.generation(identity)?;
        for name in state.inventory.keys() {
            fs::atomic(
                &working.join(name),
                &fs::read(&previous.join(name), 2 * 1024 * 1024)?,
                0o600,
            )?;
        }
    }
    Ok(working)
}

pub(crate) fn publish_cache(store: &Store, state: &mut Journal, working: &Path) -> Result<()> {
    let mut files = BTreeMap::new();
    for name in [
        "root.json",
        "timestamp.json",
        "snapshot.json",
        "targets.json",
    ] {
        if working.join(name).try_exists()? {
            let bytes = fs::read(&working.join(name), 2 * 1024 * 1024)?;
            let _: serde_json::Value = serde_json::from_slice(&bytes)?;
            files.insert(name, bytes);
        }
    }
    if !files.contains_key("root.json") {
        return Err(UpdateError::Recovery("trusted root was not persisted"));
    }
    let hashes: BTreeMap<String, String> = files
        .iter()
        .map(|(name, bytes)| (name.to_string(), fs::digest(bytes)))
        .collect();
    if state.cache.is_some() && hashes == state.inventory {
        return store.save(state);
    }
    let snapshot = store.new_generation()?;
    for (name, bytes) in files {
        fs::atomic(&snapshot.join(name), &bytes, 0o600)?;
    }
    fs::sync_dir(&snapshot)?;
    fs::sync_dir(
        snapshot
            .parent()
            .ok_or(UpdateError::Recovery("missing generation parent"))?,
    )?;
    state.inventory = inventory(&snapshot)?;
    state.cache = snapshot
        .file_name()
        .and_then(|name| name.to_str())
        .map(str::to_owned);
    store.save(state)
}

async fn repository(
    policy: &Policy,
    working: &Path,
    bootstrap: &[u8],
    http: Http,
) -> Result<Repository> {
    let root = if working.join("root.json").exists() {
        fs::read(&working.join("root.json"), 2 * 1024 * 1024)?
    } else {
        bootstrap.to_vec()
    };
    let loaded = RepositoryLoader::new(
        &root,
        policy.metadata_url.clone(),
        policy.targets_url.clone(),
    )
    .datastore(working)
    .transport(http.clone())
    .limits(Limits {
        max_root_size: 262144,
        max_timestamp_size: 16384,
        max_snapshot_size: 262144,
        max_targets_size: 2 * 1024 * 1024,
        max_root_updates: 8,
    })
    .load()
    .await?;
    check_transport(&http)?;
    Ok(loaded)
}

fn accept(
    repository: &Repository,
    policy: &Policy,
    state: &mut Journal,
) -> Result<(TargetName, u64, Artifact)> {
    let name = TargetName::new(policy.target_name())?;
    let target = repository
        .targets()
        .signed
        .targets
        .get(&name)
        .ok_or(UpdateError::Policy("fixed target is missing"))?;
    let release: Release = serde_json::from_value(serde_json::to_value(&target.custom)?)?;
    release.validate(policy)?;
    let hash = hex::encode(&target.hashes.sha256);
    sanitize_digest(&hash)?;
    if target.length == 0
        || target.length > PAYLOAD_LIMIT as u64
        || !repository.root().signed.consistent_snapshot
        || release.release_sequence < state.seen_sequence
        || release.release_sequence == state.seen_sequence && hash != state.seen_digest
    {
        return Err(UpdateError::Policy(
            "release downgrade, equivocation or size rejected",
        ));
    }
    if release.min_rollback_sequence > state.installed.sequence {
        return Err(UpdateError::Policy(
            "baseline revoked: manual upgrade required",
        ));
    }
    state.seen_sequence = release.release_sequence;
    state.seen_digest = hash.clone();
    state.hard_floor = state
        .hard_floor
        .max(release.min_rollback_sequence)
        .max(policy.minimum_sequence);
    state.time_high = epoch()?.max(state.time_high);
    Ok((
        name,
        target.length,
        Artifact {
            digest: hash,
            sequence: release.release_sequence,
        },
    ))
}

pub(crate) async fn fetch(
    store: &Store,
    state: &mut Journal,
    policy: &Policy,
    bootstrap: &[u8],
) -> Result<Artifact> {
    if epoch()?.saturating_add(5) < state.time_high {
        return Err(UpdateError::Policy(
            "wall clock precedes accepted time floor",
        ));
    }
    let working = stage(store, state)?;
    let progress = Arc::new(crate::trust::Progress::new(
        store,
        state,
        working.clone(),
        bootstrap,
    )?);
    let http = Http::new(policy, progress.clone())?;
    let loaded = repository(policy, &working, bootstrap, http).await;
    *state = progress.finish()?;
    let repository = match loaded {
        Ok(repository) => repository,
        Err(error) => {
            return Err(error);
        }
    };
    publish_cache(store, state, &working)?;
    let (name, length, artifact) = accept(&repository, policy, state)?;
    publish_cache(store, state, &working)?;
    crate::fault::checkpoint("cache_committed")?;
    let stream = repository
        .read_target(&name)
        .await?
        .ok_or(UpdateError::Policy("fixed target is missing"))?;
    let bytes = stream
        .try_fold(Vec::new(), |mut bytes, chunk| async move {
            bytes.extend_from_slice(&chunk);
            Ok(bytes)
        })
        .await?;
    if bytes.len() != length as usize || fs::digest(&bytes) != artifact.digest {
        return Err(UpdateError::Policy("downloaded executable mismatch"));
    }
    fs::atomic(&store.object(&artifact.digest)?, &bytes, 0o550)?;
    crate::fault::checkpoint("artifact_durable")?;
    Ok(artifact)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn release_fields_are_not_extensible_commands() {
        assert!(serde_json::from_str::<Release>(r#"{"product":"nodelite","component":"agent","channel":"stable","target":"x86_64-unknown-linux-musl","release_sequence":2,"min_rollback_sequence":1,"version":"3.0.4","command":"sh"}"#).is_err());
    }
}
