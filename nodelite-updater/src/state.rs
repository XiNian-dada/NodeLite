//! One durable journal separates accepted trust, installed code and the pending trial.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::error::{Result, UpdateError};
use crate::fs::{self, PAYLOAD_LIMIT};
use crate::policy::{Policy, sanitize_digest, sanitize_sequence};

pub(crate) const HOME: &str = "/var/lib/nodelite-updater";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Artifact {
    pub digest: String,
    pub sequence: u64,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Phase {
    Stable,
    Prepared,
    Stopping,
    Trial,
    RollbackIntent,
    Committed,
    RolledBack,
    ManualRecovery,
}

impl Phase {
    pub(crate) fn terminal(self) -> bool {
        matches!(self, Self::Stable | Self::Committed | Self::RolledBack)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Transaction {
    pub from: Artifact,
    pub to: Artifact,
    pub boot: String,
    pub deadline_ms: u64,
    pub old_invocation: String,
    pub bridge_exit: String,
    pub rollback_consumed: bool,
    pub recovery_requested: bool,
    pub stable_since_ms: Option<u64>,
    pub stable_sample_ms: Option<u64>,
    pub stable_invocation: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Journal {
    pub schema: u32,
    pub anchor: String,
    pub channel: String,
    pub target: String,
    pub phase: Phase,
    pub installed: Artifact,
    pub seen_sequence: u64,
    pub seen_digest: String,
    pub hard_floor: u64,
    pub time_high: u64,
    pub cache: Option<String>,
    pub inventory: BTreeMap<String, String>,
    pub failed: Vec<String>,
    pub transaction: Option<Transaction>,
}

pub(crate) struct Store {
    pub root: PathBuf,
}

pub(crate) fn epoch() -> Result<u64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .map_err(|_| UpdateError::Recovery("wall clock precedes Unix epoch"))
}

impl Store {
    pub(crate) fn system() -> Self {
        Self { root: HOME.into() }
    }
    pub(crate) fn private(&self) -> PathBuf {
        self.root.join("private")
    }
    pub(crate) fn code(&self) -> PathBuf {
        self.root.join("code")
    }
    pub(crate) fn object(&self, hash: &str) -> Result<PathBuf> {
        sanitize_digest(hash)?;
        Ok(self.code().join("objects").join(hash))
    }
    pub(crate) fn save(&self, state: &Journal) -> Result<()> {
        fs::json(&self.private().join("journal.json"), state)
    }

    pub(crate) fn initialize(&self, policy: &Policy, root: &[u8], baseline: &Path) -> Result<()> {
        if std::fs::read_dir(self.private())?
            .any(|entry| entry.map_or(true, |entry| entry.file_name() != "lock"))
        {
            return Err(UpdateError::Recovery(
                "refusing to reinitialize existing update state",
            ));
        }
        let payload = fs::read(baseline, PAYLOAD_LIMIT)?;
        if fs::digest(&payload) != policy.baseline_sha256 {
            return Err(UpdateError::Policy(
                "manually enrolled baseline hash mismatch",
            ));
        }
        fs::directory(&self.private().join("generations"), 0o700)?;
        fs::atomic(&self.object(&policy.baseline_sha256)?, &payload, 0o550)?;
        fs::pointer(&self.code(), "active", &policy.baseline_sha256)?;
        fs::pointer(&self.code(), "candidate", &policy.baseline_sha256)?;
        let journal = Journal {
            schema: 1,
            anchor: fs::digest(root),
            channel: policy.channel.clone(),
            target: policy.target.clone(),
            phase: Phase::Stable,
            installed: Artifact {
                digest: policy.baseline_sha256.clone(),
                sequence: policy.baseline_sequence,
            },
            seen_sequence: policy.baseline_sequence,
            seen_digest: policy.baseline_sha256.clone(),
            hard_floor: policy.minimum_sequence,
            time_high: epoch()?,
            cache: None,
            inventory: BTreeMap::new(),
            failed: Vec::new(),
            transaction: None,
        };
        self.save(&journal)?;
        fs::sync_dir(&self.private())
    }

    pub(crate) fn load(&self, policy: &Policy, anchor: &[u8]) -> Result<Journal> {
        let state: Journal =
            serde_json::from_slice(&fs::read(&self.private().join("journal.json"), 65536)?)?;
        state.validate(policy, anchor)?;
        if let Some(generation) = &state.cache {
            if inventory(&self.generation(generation)?)? != state.inventory {
                return Err(UpdateError::Recovery("committed trust cache changed"));
            }
        } else if !state.inventory.is_empty() {
            return Err(UpdateError::Recovery("trust cache generation is missing"));
        }
        self.verify_object(&state.installed.digest)?;
        if let Some(tx) = &state.transaction {
            self.verify_object(&tx.from.digest)?;
            self.verify_object(&tx.to.digest)?;
        }
        Ok(state)
    }

    pub(crate) fn generation(&self, identity: &str) -> Result<PathBuf> {
        if identity.len() != 32
            || !identity
                .bytes()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
        {
            return Err(UpdateError::Recovery("invalid trust generation"));
        }
        let path = self.private().join("generations").join(identity);
        if path.exists() && !std::fs::symlink_metadata(&path)?.is_dir() {
            return Err(UpdateError::Recovery("unsafe trust generation"));
        }
        Ok(path)
    }

    pub(crate) fn verify_object(&self, hash: &str) -> Result<()> {
        if fs::digest(&fs::read(&self.object(hash)?, PAYLOAD_LIMIT)?) != hash {
            return Err(UpdateError::Recovery("committed executable changed"));
        }
        Ok(())
    }

    pub(crate) fn active(&self) -> Result<String> {
        let link = std::fs::read_link(self.code().join("active"))?;
        let text = link
            .to_str()
            .ok_or(UpdateError::Recovery("invalid active link"))?;
        let hash = text
            .strip_prefix("objects/")
            .ok_or(UpdateError::Recovery("invalid active link"))?;
        self.verify_object(hash)?;
        Ok(hash.into())
    }
}

impl Journal {
    fn validate(&self, policy: &Policy, anchor: &[u8]) -> Result<()> {
        if self.schema != 1
            || self.anchor != fs::digest(anchor)
            || self.channel != policy.channel
            || self.target != policy.target
            || self.failed.len() > 64
            || self.time_high == 0
        {
            return Err(UpdateError::Recovery("journal scope or schema rejected"));
        }
        for digest in [&self.installed.digest, &self.seen_digest]
            .into_iter()
            .chain(self.failed.iter())
        {
            sanitize_digest(digest)?;
        }
        for seq in [self.installed.sequence, self.seen_sequence, self.hard_floor] {
            sanitize_sequence(seq)?;
        }
        if self.installed.sequence > self.seen_sequence || self.hard_floor > self.installed.sequence
        {
            return Err(UpdateError::Recovery(
                "installed release is outside its trust floor",
            ));
        }
        if !self.phase.terminal()
            && self.phase != Phase::ManualRecovery
            && self.transaction.is_none()
        {
            return Err(UpdateError::Recovery("pending transaction is missing"));
        }
        if let Some(tx) = &self.transaction {
            for item in [&tx.from, &tx.to] {
                sanitize_digest(&item.digest)?;
                sanitize_sequence(item.sequence)?;
            }
            if tx.to.sequence <= tx.from.sequence
                || tx.from != self.installed && self.phase != Phase::Committed
                || tx.to.sequence > self.seen_sequence
                || tx.from.sequence < self.hard_floor
            {
                return Err(UpdateError::Recovery(
                    "inconsistent transaction authorization",
                ));
            }
        }
        Ok(())
    }
}

pub(crate) fn inventory(path: &Path) -> Result<BTreeMap<String, String>> {
    let mut result = BTreeMap::new();
    for entry in std::fs::read_dir(path)? {
        let entry = entry?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| UpdateError::Recovery("invalid cache filename"))?;
        if !matches!(
            name.as_str(),
            "root.json"
                | "timestamp.json"
                | "snapshot.json"
                | "targets.json"
                | "latest_known_time.json"
        ) {
            return Err(UpdateError::Recovery("unexpected cache file"));
        }
        result.insert(name, fs::digest(&fs::read(&entry.path(), 2 * 1024 * 1024)?));
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_inventory_rejects_unknown_files_and_symlinks() {
        let dir = tempfile::tempdir().expect("temporary directory");
        std::fs::write(dir.path().join("root.json"), b"root").expect("write");
        assert_eq!(inventory(dir.path()).expect("inventory").len(), 1);
        std::fs::write(dir.path().join("unknown"), b"unexpected").expect("write");
        assert!(inventory(dir.path()).is_err());
        std::fs::remove_file(dir.path().join("unknown")).expect("remove");
        std::os::unix::fs::symlink("root.json", dir.path().join("targets.json")).expect("symlink");
        assert!(inventory(dir.path()).is_err());
    }

    fn fixture() -> (tempfile::TempDir, Store, Policy, Journal) {
        let directory = tempfile::tempdir().expect("directory");
        let store = Store {
            root: directory.path().to_path_buf(),
        };
        for path in [store.private(), store.code(), store.code().join("objects")] {
            fs::directory(&path, 0o700).expect("directory");
        }
        let policy = crate::policy::fixture();
        let baseline = directory.path().join("baseline");
        std::fs::write(&baseline, b"baseline").expect("baseline");
        store
            .initialize(&policy, b"public-root", &baseline)
            .expect("initialize");
        let journal = store.load(&policy, b"public-root").expect("load");
        (directory, store, policy, journal)
    }

    #[test]
    fn enrolled_baseline_cannot_be_reinitialized_or_silently_replaced() {
        let (directory, store, policy, _) = fixture();
        assert!(
            store
                .initialize(&policy, b"public-root", &directory.path().join("baseline"))
                .is_err()
        );
        std::fs::write(store.private().join("journal.json"), b"{").expect("corrupt");
        assert!(store.load(&policy, b"public-root").is_err());
        std::fs::remove_file(store.private().join("journal.json")).expect("remove");
        assert!(store.load(&policy, b"public-root").is_err());
    }

    #[test]
    fn altered_objects_anchors_and_scope_require_recovery() {
        let (_directory, store, mut policy, journal) = fixture();
        assert!(store.load(&policy, b"other-root").is_err());
        policy.channel = "testing".into();
        assert!(store.load(&policy, b"public-root").is_err());
        policy.channel = "stable".into();
        fs::atomic(
            &store.object(&journal.installed.digest).expect("object"),
            b"changed",
            0o550,
        )
        .expect("corrupt");
        assert!(store.load(&policy, b"public-root").is_err());
    }

    #[test]
    fn impossible_state_combinations_are_rejected() {
        let (_directory, _store, policy, mut journal) = fixture();
        journal.phase = Phase::Trial;
        assert!(journal.validate(&policy, b"public-root").is_err());
        journal.phase = Phase::Stable;
        journal.hard_floor = 2;
        assert!(journal.validate(&policy, b"public-root").is_err());
        journal.hard_floor = 1;
        journal.seen_sequence = 0;
        assert!(journal.validate(&policy, b"public-root").is_err());
    }
}
