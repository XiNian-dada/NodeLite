//! Only unreferenced, flat updater-owned caches may be collected under the transaction lock.

use std::collections::BTreeSet;
use std::os::unix::fs::MetadataExt;
use std::path::Path;

use crate::error::{Result, UpdateError};
use crate::fs;
use crate::state::{Journal, Store};

fn temporary(name: &str) -> bool {
    name.strip_prefix(".update-")
        .is_some_and(|suffix| suffix.len() == 32 && suffix.bytes().all(|b| b.is_ascii_hexdigit()))
}

fn owned_file(path: &Path, executable: bool) -> Result<()> {
    let metadata = path.symlink_metadata()?;
    // GC cannot reinterpret another user's inode as an updater-created cache.
    let uid = unsafe { libc::geteuid() };
    let mode = if executable { 0o550 } else { 0o600 };
    if !metadata.is_file()
        || metadata.uid() != uid
        || metadata.nlink() != 1
        || metadata.mode() & 0o7777 != mode
    {
        return Err(UpdateError::Recovery("unsafe unreferenced cache inode"));
    }
    Ok(())
}

fn generations(store: &Store, current: Option<&str>) -> Result<()> {
    for entry in std::fs::read_dir(store.private().join("generations"))? {
        let entry = entry?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| UpdateError::Recovery("invalid cache entry"))?;
        let path = store.generation(&name)?;
        if Some(name.as_str()) == current {
            continue;
        }
        for file in std::fs::read_dir(&path)? {
            let file = file?;
            let name = file
                .file_name()
                .into_string()
                .map_err(|_| UpdateError::Recovery("invalid cache entry"))?;
            if !matches!(
                name.as_str(),
                "root.json"
                    | "timestamp.json"
                    | "snapshot.json"
                    | "targets.json"
                    | "latest_known_time.json"
            ) && !temporary(&name)
            {
                return Err(UpdateError::Recovery("unexpected unreferenced cache entry"));
            }
            owned_file(&file.path(), false)?;
            std::fs::remove_file(file.path())?;
        }
        std::fs::remove_dir(path)?;
    }
    fs::sync_dir(&store.private().join("generations"))
}

pub(crate) fn collect(store: &Store, state: &Journal) -> Result<()> {
    if !state.phase.terminal() {
        return Err(UpdateError::Recovery("cannot collect during activation"));
    }
    generations(store, state.cache.as_deref())?;
    let mut pins = BTreeSet::from([state.installed.digest.as_str(), state.seen_digest.as_str()]);
    if let Some(transaction) = &state.transaction {
        pins.extend([
            transaction.from.digest.as_str(),
            transaction.to.digest.as_str(),
        ]);
    }
    for entry in std::fs::read_dir(store.code().join("objects"))? {
        let entry = entry?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| UpdateError::Recovery("invalid object entry"))?;
        if !temporary(&name) {
            crate::policy::sanitize_digest(&name)?;
        }
        if pins.contains(name.as_str()) {
            continue;
        }
        owned_file(&entry.path(), true)?;
        std::fs::remove_file(entry.path())?;
    }
    fs::sync_dir(&store.code().join("objects"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_unreferenced_generations_are_removed() {
        let directory = tempfile::tempdir().expect("directory");
        let store = Store {
            root: directory.path().to_path_buf(),
        };
        std::fs::create_dir_all(store.private().join("generations")).expect("parents");
        let old = store.new_generation().expect("old");
        let current = store.new_generation().expect("current");
        fs::atomic(&old.join("root.json"), b"old", 0o600).expect("old metadata");
        fs::atomic(&current.join("root.json"), b"current", 0o600).expect("current metadata");
        generations(&store, current.file_name().and_then(|value| value.to_str())).expect("collect");
        assert!(!old.exists());
        assert_eq!(
            fs::read(&current.join("root.json"), 20).expect("read"),
            b"current"
        );
    }

    #[test]
    fn cache_links_cannot_redirect_collection() {
        let directory = tempfile::tempdir().expect("directory");
        let store = Store {
            root: directory.path().to_path_buf(),
        };
        std::fs::create_dir_all(store.private().join("generations")).expect("parents");
        let old = store.new_generation().expect("old");
        let outside = directory.path().join("outside");
        std::fs::write(&outside, b"preserve").expect("outside");
        std::os::unix::fs::symlink(&outside, old.join("root.json")).expect("link");
        assert!(generations(&store, None).is_err());
        assert_eq!(std::fs::read(outside).expect("preserved"), b"preserve");
    }
}
