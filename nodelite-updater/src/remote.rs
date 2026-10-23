//! A fixed non-root service consumes untrusted triggers and publishes credential-free status.

use std::os::unix::fs::MetadataExt;
use std::time::Duration;

use nodelite_proto::agent_update::{
    AgentUpdateCapability as Capability, AgentUpdatePhase as UpdatePhase,
    AgentUpdateReason as Reason, AgentUpdateRequest, AgentUpdateStatus, UPDATE_COOLDOWN_SECS,
};
use serde::{Deserialize, Serialize};

use crate::error::{Result, UpdateError};
use crate::fs::Lock;
use crate::policy::Policy;
use crate::state::{Artifact, Journal, Phase, Store, epoch};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RemoteUpdate {
    request: AgentUpdateRequest,
    accepted_at: u64,
    target: Option<String>,
    status: AgentUpdateStatus,
}

pub(crate) fn validate(state: &Journal) -> Result<()> {
    if let Some(update) = &state.remote_update {
        if !update.request.validate(update.accepted_at)
            || !update.status.validate()
            || update.status.request_id.as_deref() != Some(update.request.request_id.as_str())
        {
            return Err(UpdateError::Recovery("invalid remote request journal"));
        }
        if let Some(target) = &update.target {
            crate::policy::sanitize_digest(target)?;
        }
    }
    Ok(())
}

pub(crate) fn snapshot(state: &mut Journal) {
    let status = status(state);
    // Preserve the completed result even when a later local upgrade starts another transaction.
    if let Some(update) = &mut state.remote_update {
        update.status = status;
    }
}

pub(crate) fn recover_download(store: &Store, state: &mut Journal) -> Result<()> {
    if state.phase.terminal()
        && let Some(update) = &mut state.remote_update
        && update.target.is_none()
        && update.status.phase == UpdatePhase::Downloading
    {
        update.status.phase = UpdatePhase::Failed;
        update.status.reason = Some(Reason::UpdateFailed);
        store.save(state)?;
    }
    Ok(())
}

pub(crate) fn up_to_date(state: &mut Journal) {
    if let Some(update) = &mut state.remote_update
        && update.target.is_none()
        && update.status.phase == UpdatePhase::Downloading
    {
        update.status.phase = UpdatePhase::UpToDate;
    }
}

pub(crate) fn bind(state: &mut Journal, to: &Artifact) {
    if let Some(update) = &mut state.remote_update
        && update.target.is_none()
        && update.status.phase == UpdatePhase::Downloading
    {
        update.target = Some(to.digest.clone());
        update.status.target_sequence = Some(to.sequence);
    }
}

pub(crate) fn status(state: &Journal) -> AgentUpdateStatus {
    let mut status = state
        .remote_update
        .as_ref()
        .map(|update| update.status.clone())
        .unwrap_or(AgentUpdateStatus {
            capability: Capability::Ready,
            phase: UpdatePhase::Idle,
            request_id: None,
            updated_at: 0,
            next_request_at: 0,
            installed_sequence: None,
            target_sequence: None,
            channel: None,
            reason: None,
        });
    status.installed_sequence = Some(state.installed.sequence);
    status.next_request_at = state.remote_update.as_ref().map_or(0, |update| {
        update.accepted_at.saturating_add(UPDATE_COOLDOWN_SECS)
    });
    status.channel = Some(state.channel.clone());
    if let Some(update) = &state.remote_update
        && let Some(transaction) = &state.transaction
        && update.target.as_deref() == Some(transaction.to.digest.as_str())
    {
        status.phase = match state.phase {
            Phase::Prepared => UpdatePhase::Prepared,
            Phase::Stopping => UpdatePhase::Stopping,
            Phase::Trial => UpdatePhase::Trial,
            Phase::Committed => UpdatePhase::Committed,
            Phase::RolledBack => UpdatePhase::RolledBack,
            Phase::RollbackIntent => UpdatePhase::RollingBack,
            Phase::ManualRecovery => UpdatePhase::ManualRecovery,
            Phase::Stable => status.phase,
        };
    }
    if state.phase == Phase::ManualRecovery {
        status.capability = Capability::ManualRecovery;
        status.phase = UpdatePhase::ManualRecovery;
        status.reason = Some(Reason::ManualRecovery);
    } else if state.remote_update.is_none() && !state.phase.terminal() {
        status.capability = Capability::Disabled;
        status.reason = Some(Reason::Busy);
    }
    status
}

pub(crate) fn publish(store: &Store, state: &Journal) -> Result<()> {
    publish_status(store, status(state))
}

pub(crate) fn publish_manual(store: &Store) -> Result<()> {
    let mut status =
        AgentUpdateStatus::unavailable(Capability::ManualRecovery, Reason::ManualRecovery);
    status.phase = UpdatePhase::ManualRecovery;
    publish_status(store, status)
}

fn publish_status(store: &Store, mut status: AgentUpdateStatus) -> Result<()> {
    let directory = store.root.join("public");
    if !directory.exists() {
        return Ok(());
    }
    let info = directory.symlink_metadata()?;
    // SAFETY: identity queries have no caller-provided memory or preconditions.
    let (uid, gid) = unsafe { (libc::geteuid(), libc::getegid()) };
    if !info.is_dir() || info.uid() != uid || info.gid() != gid || info.mode() & 0o7777 != 0o750 {
        return Err(UpdateError::Recovery("unsafe public status directory"));
    }
    status.updated_at = epoch()?;
    crate::fs::atomic(
        &directory.join("status.json"),
        &serde_json::to_vec(&status)?,
        0o640,
    )
}

fn consume(store: &Store) -> Result<AgentUpdateRequest> {
    let source = store.root.join("requests/request.json");
    let consumed = store.private().join("remote-request.json");
    std::fs::rename(source, &consumed)?;
    crate::fs::sync_dir(&store.root.join("requests"))?;
    let bytes = crate::fs::read(&consumed, 1024);
    std::fs::remove_file(&consumed)?;
    crate::fs::sync_dir(&store.private())?;
    Ok(serde_json::from_slice(&bytes?)?)
}

pub(crate) async fn take(store: &Store) -> Result<(AgentUpdateRequest, Lock)> {
    // Claim before waiting: a persistent PathExists trigger would otherwise exhaust systemd's
    // start limit on a momentarily busy lock. No trust or journal changes precede the lock.
    let request = consume(store)?;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        match Lock::acquire(&store.private().join("lock")) {
            Ok(lock) => return Ok((request, lock)),
            Err(UpdateError::Busy) if tokio::time::Instant::now() < deadline => {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            Err(error) => return Err(error),
        }
    }
}

fn accept(
    state: &mut Journal,
    request: AgentUpdateRequest,
    policy: &Policy,
    now: u64,
) -> Result<()> {
    if !policy.allow_remote_updates || !request.validate(now) {
        return Err(UpdateError::Policy(
            "remote update policy or request rejected",
        ));
    }
    if !state.phase.terminal() {
        return Err(UpdateError::Busy);
    }
    if let Some(previous) = &state.remote_update
        && (previous.request.request_id == request.request_id
            || now < previous.accepted_at.saturating_add(UPDATE_COOLDOWN_SECS))
    {
        return Err(UpdateError::Policy("remote request replay or cooldown"));
    }
    let mut status = status(state);
    status.phase = UpdatePhase::Downloading;
    status.request_id = Some(request.request_id.clone());
    status.reason = None;
    status.updated_at = now;
    status.target_sequence = None;
    state.remote_update = Some(RemoteUpdate {
        request,
        accepted_at: now,
        target: None,
        status,
    });
    Ok(())
}

pub(crate) async fn run(
    store: &Store,
    policy: &Policy,
    bootstrap: &[u8],
    request: AgentUpdateRequest,
) -> Result<()> {
    let mut state = store.load(policy, bootstrap).inspect_err(|_| {
        let _ = crate::system::manual(store, "journal or committed object invalid");
    })?;
    if store.private().join("recovery-required.json").exists() {
        return Err(UpdateError::Recovery("operator recovery required"));
    }
    accept(&mut state, request, policy, epoch()?)?;
    crate::fs::atomic(&store.private().join("pending"), b"pending\n", 0o600)?;
    store.save(&state)?;
    let result = crate::runtime::apply(store, policy, bootstrap).await;
    let mut state = store.load(policy, bootstrap)?;
    if let Some(update) = &mut state.remote_update {
        if result.is_err() {
            update.status.phase = UpdatePhase::Failed;
            update.status.reason = Some(Reason::UpdateFailed);
        } else if update.target.is_none() {
            update.status.phase = UpdatePhase::UpToDate;
        }
    }
    store.save(&state)?;
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> (tempfile::TempDir, Store, Policy, Journal) {
        let dir = tempfile::tempdir().expect("directory");
        let store = Store {
            root: dir.path().into(),
        };
        for path in [store.private(), store.code(), store.code().join("objects")] {
            std::fs::create_dir_all(path).expect("layout");
        }
        let policy = crate::policy::fixture();
        let baseline = dir.path().join("baseline");
        std::fs::write(&baseline, b"baseline").expect("baseline");
        store
            .initialize(&policy, b"root", &baseline)
            .expect("initialize");
        let state = serde_json::from_slice(
            &crate::fs::read(&store.private().join("journal.json"), 65536).expect("journal"),
        )
        .expect("state");
        (dir, store, policy, state)
    }

    #[test]
    fn local_opt_in_expiry_replay_cooldown_and_pending_transaction_are_enforced() {
        let (_dir, _store, mut policy, mut state) = state();
        let request = AgentUpdateRequest {
            request_id: "a".repeat(32),
            issued_at: 100,
            expires_at: 160,
        };
        assert!(accept(&mut state, request.clone(), &policy, 100).is_err());
        policy.allow_remote_updates = true;
        assert!(accept(&mut state, request.clone(), &policy, 160).is_err());
        accept(&mut state, request.clone(), &policy, 100).expect("authorized request");
        assert!(accept(&mut state, request.clone(), &policy, 100).is_err());
        let next = AgentUpdateRequest {
            request_id: "b".repeat(32),
            issued_at: 400,
            expires_at: 460,
        };
        state.phase = Phase::Trial;
        assert!(accept(&mut state, next.clone(), &policy, 400).is_err());
        state.phase = Phase::Committed;
        accept(&mut state, next, &policy, 400).expect("cooldown elapsed");
        assert_eq!(status(&state).phase, UpdatePhase::Downloading);
    }

    #[test]
    fn old_journals_remain_readable_and_requests_cannot_include_commands() {
        let (_dir, store, _policy, state) = state();
        let mut value = serde_json::to_value(&state).expect("JSON");
        value
            .as_object_mut()
            .expect("object")
            .remove("remote_update");
        let old: Journal = serde_json::from_value(value).expect("old journal");
        assert!(old.remote_update.is_none());
        std::fs::create_dir(store.root.join("requests")).expect("requests");
        std::fs::write(
            store.root.join("requests/request.json"),
            b"{\"command\":\"sh\"}",
        )
        .expect("malformed request");
        assert!(consume(&store).is_err());
        assert!(!store.root.join("requests/request.json").exists());
    }

    #[tokio::test]
    async fn a_busy_transaction_cannot_retrigger_the_unconsumed_path() {
        let (_dir, store, _policy, _state) = state();
        std::fs::create_dir(store.root.join("requests")).expect("requests");
        let source = store.root.join("requests/request.json");
        let request = AgentUpdateRequest {
            request_id: "d".repeat(32),
            issued_at: 100,
            expires_at: 160,
        };
        std::fs::write(&source, serde_json::to_vec(&request).expect("JSON")).expect("request");
        let held = Lock::acquire(&store.private().join("lock")).expect("held lock");
        let (claimed, ()) = tokio::join!(take(&store), async {
            tokio::time::sleep(Duration::from_millis(25)).await;
            assert!(
                !source.exists(),
                "the path must be consumed even while the lock is busy"
            );
            drop(held);
        });
        let (received, _lock) = claimed.expect("wait for brief contention");
        assert_eq!(received, request);
    }

    #[tokio::test]
    async fn corrupt_remote_journal_marks_and_publishes_manual_recovery() {
        use std::os::unix::fs::PermissionsExt;

        let (_dir, store, policy, _state) = state();
        let public = store.root.join("public");
        std::fs::create_dir(&public).expect("public directory");
        std::fs::set_permissions(&public, std::fs::Permissions::from_mode(0o750))
            .expect("public mode");
        std::fs::write(store.private().join("journal.json"), b"corrupt").expect("damage journal");
        let result = run(
            &store,
            &policy,
            b"root",
            AgentUpdateRequest {
                request_id: "e".repeat(32),
                issued_at: 100,
                expires_at: 160,
            },
        )
        .await;
        assert!(result.is_err());
        assert!(store.private().join("recovery-required.json").is_file());
        let published: AgentUpdateStatus = serde_json::from_slice(
            &crate::fs::read(&public.join("status.json"), 4096).expect("status"),
        )
        .expect("published status");
        assert_eq!(published.capability, Capability::ManualRecovery);
        assert_eq!(published.phase, UpdatePhase::ManualRecovery);
    }

    #[test]
    fn cooldown_uses_acceptance_time_not_later_progress_publication() {
        let (_dir, store, mut policy, mut state) = state();
        policy.allow_remote_updates = true;
        accept(
            &mut state,
            AgentUpdateRequest {
                request_id: "c".repeat(32),
                issued_at: 100,
                expires_at: 160,
            },
            &policy,
            100,
        )
        .expect("request");
        state
            .remote_update
            .as_mut()
            .expect("remote job")
            .status
            .updated_at = 390;
        assert_eq!(status(&state).next_request_at, 400);
        recover_download(&store, &mut state).expect("interrupted download");
        assert_eq!(status(&state).phase, UpdatePhase::Failed);
        assert_eq!(status(&state).next_request_at, 400);
    }
}
