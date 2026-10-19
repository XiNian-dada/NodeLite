//! Persist verified root progress before the next network step; replay it through TUF itself.

use std::path::PathBuf;
use std::sync::Mutex;

use crate::error::{Result, UpdateError};
use crate::fs;
use crate::state::{Journal, Store};

pub(crate) fn root_version(bytes: &[u8]) -> Result<u64> {
    let root: tough::schema::Signed<tough::schema::Root> = serde_json::from_slice(bytes)?;
    root.signed
        .verify_role(&root)
        .map_err(|_| UpdateError::Recovery("cached root self-signatures rejected"))?;
    Ok(root.signed.version.get())
}

pub(crate) struct Progress {
    store: Store,
    working: PathBuf,
    inner: Mutex<(Journal, bool)>,
    initial: Vec<u8>,
}

impl Progress {
    pub(crate) fn new(
        store: &Store,
        state: &Journal,
        working: PathBuf,
        bootstrap: &[u8],
    ) -> Result<Self> {
        let initial = if working.join("root.json").exists() {
            fs::read(&working.join("root.json"), 262144)?
        } else {
            bootstrap.to_vec()
        };
        Ok(Self {
            store: Store {
                root: store.root.clone(),
            },
            working,
            inner: Mutex::new((state.clone(), false)),
            initial,
        })
    }

    fn capture_root(&self, state: &mut Journal) -> Result<()> {
        let path = self.working.join("root.json");
        if !path.try_exists()? {
            return Ok(());
        }
        let bytes = fs::read(&path, 262144)?;
        let version = root_version(&bytes)?;
        if state.roots.contains_key(&version) {
            return Ok(());
        }
        self.verify_continuity(state, &bytes, version)?;
        if state.roots.len() >= 128 {
            return Err(UpdateError::Recovery("root history quota reached"));
        }
        let hash = fs::digest(&bytes);
        fs::atomic(
            &self.store.private().join("roots").join(&hash),
            &bytes,
            0o600,
        )?;
        state.roots.insert(version, hash);
        self.store.save(state)?;
        crate::fault::checkpoint("root_persisted")
    }

    fn verify_continuity(&self, state: &Journal, bytes: &[u8], version: u64) -> Result<()> {
        let initial_version = root_version(&self.initial)?;
        let previous = match state.roots.range(..version).next_back() {
            Some((previous, hash)) if *previous >= initial_version => {
                self.store.root_bytes(hash)?
            }
            _ => self.initial.clone(),
        };
        let previous: tough::schema::Signed<tough::schema::Root> =
            serde_json::from_slice(&previous)?;
        let next: tough::schema::Signed<tough::schema::Root> = serde_json::from_slice(bytes)?;
        if version == previous.signed.version.get() {
            if serde_json::to_value(&previous.signed)? != serde_json::to_value(&next.signed)? {
                return Err(UpdateError::Recovery("cached root role equivocation"));
            }
        } else if version != previous.signed.version.get().saturating_add(1) {
            return Err(UpdateError::Recovery("cached root chain has a gap"));
        }
        previous
            .signed
            .verify_role(&next)
            .map_err(|_| UpdateError::Recovery("cached root old threshold rejected"))?;
        Ok(())
    }

    pub(crate) fn before(&self, name: &str) -> Result<Option<Vec<u8>>> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| UpdateError::Recovery("trust progress lock poisoned"))?;
        self.capture_root(&mut inner.0)?;
        if name == "timestamp.json" {
            inner.1 = true;
        }
        if inner.1 {
            crate::download::publish_cache(&self.store, &mut inner.0, &self.working)?;
        }
        if let Some(version) = name
            .strip_suffix(".root.json")
            .and_then(|value| value.parse::<u64>().ok())
            && let Some(hash) = inner.0.roots.get(&version)
        {
            return Ok(Some(self.store.root_bytes(hash)?));
        }
        Ok(None)
    }

    pub(crate) fn finish(&self) -> Result<Journal> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| UpdateError::Recovery("trust progress lock poisoned"))?;
        self.capture_root(&mut inner.0)?;
        if inner.1 {
            crate::download::publish_cache(&self.store, &mut inner.0, &self.working)?;
        }
        Ok(inner.0.clone())
    }
}
