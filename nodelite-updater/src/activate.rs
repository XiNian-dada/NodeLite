//! Journal-first activation, a synchronous stop barrier and one authorized rollback.

use crate::error::{Result, UpdateError};
use crate::fs;
use crate::policy::Policy;
use crate::state::{Artifact, Journal, Phase, Store, Transaction};
use crate::system::{self, AGENT, PREFLIGHT, STOP, value};
use nodelite_proto::update_health::UpdateHealth;
use std::collections::BTreeMap;

/// Host effects of the activation state machine.
///
/// Kept behind a trait so the journal transitions and fault cut points can be
/// replayed deterministically in tests without systemd (see `activate_tests`).
pub(crate) trait Host {
    async fn properties(&self, unit: &str) -> Result<BTreeMap<String, String>>;
    fn boot_id(&self) -> Result<String>;
    fn boottime_ms(&self) -> Result<u64>;
    fn marker(&self, action: &str) -> Result<()>;
    fn marker_pending(&self, name: &str) -> bool;
    fn stopped(&self, properties: &BTreeMap<String, String>) -> Result<bool>;
    fn health(
        &self,
        store: &Store,
        process: &BTreeMap<String, String>,
        expected: &str,
    ) -> Result<Option<UpdateHealth>>;
    fn checkpoint(&self, name: &str) -> Result<()>;
}

pub(crate) struct SystemHost;

impl Host for SystemHost {
    async fn properties(&self, unit: &str) -> Result<BTreeMap<String, String>> {
        system::properties(unit).await
    }
    fn boot_id(&self) -> Result<String> {
        system::boot_id()
    }
    fn boottime_ms(&self) -> Result<u64> {
        system::boottime_ms()
    }
    fn marker(&self, action: &str) -> Result<()> {
        system::marker(action)
    }
    fn marker_pending(&self, name: &str) -> bool {
        system::marker_pending(name)
    }
    fn stopped(&self, properties: &BTreeMap<String, String>) -> Result<bool> {
        system::stopped(properties)
    }
    fn health(
        &self,
        store: &Store,
        process: &BTreeMap<String, String>,
        expected: &str,
    ) -> Result<Option<UpdateHealth>> {
        system::health(store, process, expected)
    }
    fn checkpoint(&self, name: &str) -> Result<()> {
        crate::fault::checkpoint(name)
    }
}

fn tx(state: &Journal) -> Result<&Transaction> {
    state
        .transaction
        .as_ref()
        .ok_or(UpdateError::Recovery("missing transaction"))
}
fn tx_mut(state: &mut Journal) -> Result<&mut Transaction> {
    state
        .transaction
        .as_mut()
        .ok_or(UpdateError::Recovery("missing transaction"))
}

pub(crate) async fn prepare(
    host: &impl Host,
    store: &Store,
    state: &mut Journal,
    policy: &Policy,
    to: Artifact,
) -> Result<()> {
    if !state.phase.terminal()
        || to.sequence <= state.installed.sequence
        || state.failed.contains(&to.digest)
        || state.failed.len() >= 64
        || state.installed.sequence < policy.minimum_sequence
        || state.installed.sequence < state.hard_floor
        || store.active()? != state.installed.digest
    {
        return Err(UpdateError::Policy(
            "activation policy or enrolled baseline rejected",
        ));
    }
    fs::pointer(&store.code(), "candidate", &to.digest)?;
    fs::atomic(&store.private().join("pending"), b"pending\n", 0o600)?;
    let old = host.properties(AGENT).await?;
    let probe = host.properties(PREFLIGHT).await?;
    crate::remote::bind(state, &to);
    state.transaction = Some(Transaction {
        from: state.installed.clone(),
        to,
        boot: host.boot_id()?,
        deadline_ms: host.boottime_ms()? + 30000,
        old_invocation: value(&old, "InvocationID").into(),
        bridge_exit: value(&probe, "ExecMainExitTimestampMonotonic").into(),
        rollback_consumed: false,
        recovery_requested: false,
        stable_since_ms: None,
        stable_sample_ms: None,
        stable_invocation: None,
    });
    state.phase = Phase::Prepared;
    store.save(state)?;
    host.checkpoint("prepared")?;
    host.marker("preflight")
}

async fn prepared(host: &impl Host, store: &Store, state: &mut Journal) -> Result<()> {
    let probe = host.properties(PREFLIGHT).await?;
    let transaction = tx(state)?;
    if transaction.boot != host.boot_id()? {
        if store.active()? != transaction.from.digest {
            return Err(UpdateError::Recovery("prepared pointer changed"));
        }
        state.phase = Phase::RolledBack;
        return store.save(state);
    }
    let done = value(&probe, "ExecMainExitTimestampMonotonic") != transaction.bridge_exit
        && !matches!(value(&probe, "ExecMainExitTimestampMonotonic"), "" | "0");
    if done && value(&probe, "Result") == "success" && value(&probe, "ActiveState") == "active" {
        let stop = host.properties(STOP).await?;
        let transaction = tx_mut(state)?;
        transaction.bridge_exit = value(&stop, "ExecMainExitTimestampMonotonic").into();
        transaction.deadline_ms = host.boottime_ms()? + 30000;
        state.phase = Phase::Stopping;
        store.save(state)?;
        host.checkpoint("stopping")?;
        host.marker("stop")?;
    } else if done || host.boottime_ms()? > transaction.deadline_ms {
        state.failed.push(transaction.to.digest.clone());
        state.phase = Phase::RolledBack;
        store.save(state)?;
    } else if !host.marker_pending("preflight")
        && !matches!(value(&probe, "ActiveState"), "activating" | "deactivating")
    {
        host.marker("preflight")?;
    }
    Ok(())
}

async fn stopping(
    host: &impl Host,
    store: &Store,
    state: &mut Journal,
    policy: &Policy,
) -> Result<()> {
    let transaction = tx(state)?;
    if transaction.boot != host.boot_id()? {
        if store.active()? != transaction.from.digest {
            return Err(UpdateError::Recovery("stopping pointer changed"));
        }
        state.phase = Phase::RolledBack;
        return store.save(state);
    }
    let agent = host.properties(AGENT).await?;
    let bridge = host.properties(STOP).await?;
    let acknowledged = value(&bridge, "ExecMainExitTimestampMonotonic") != transaction.bridge_exit
        && !matches!(value(&bridge, "ExecMainExitTimestampMonotonic"), "" | "0")
        && value(&bridge, "ActiveState") == "inactive"
        && value(&bridge, "Result") == "success";
    if acknowledged && !host.marker_pending("stop") && host.stopped(&agent)? {
        tx_mut(state)?.deadline_ms = host.boottime_ms()? + policy.trial_seconds * 1000;
        state.phase = Phase::Trial;
        store.save(state)?;
        host.checkpoint("trial_journal")?;
        fs::pointer(&store.code(), "active", &tx(state)?.to.digest)?;
        host.checkpoint("active_pointer")?;
        host.marker("restart")?;
    } else if host.boottime_ms()? > transaction.deadline_ms {
        return Err(UpdateError::Recovery("old service stop was not confirmed"));
    } else if !matches!(
        value(&bridge, "ActiveState"),
        "active" | "activating" | "deactivating"
    ) && !host.marker_pending("stop")
    {
        host.marker("stop")?;
    }
    Ok(())
}

async fn rollback(host: &impl Host, store: &Store, state: &mut Journal) -> Result<()> {
    if tx(state)?.from.sequence < state.hard_floor {
        return Err(UpdateError::Recovery("rollback baseline revoked"));
    }
    if state.phase != Phase::RollbackIntent {
        let failed = tx(state)?.to.digest.clone();
        tx_mut(state)?.rollback_consumed = true;
        tx_mut(state)?.deadline_ms = host.boottime_ms()? + 60000;
        state.failed.push(failed);
        state.phase = Phase::RollbackIntent;
        store.save(state)?;
        host.checkpoint("rollback_intent")?;
    }
    let transaction = tx(state)?;
    store.verify_object(&transaction.from.digest)?;
    let process = host.properties(AGENT).await?;
    if host
        .health(store, &process, &transaction.from.digest)?
        .is_some()
    {
        state.phase = Phase::RolledBack;
        store.save(state)?;
        return Ok(());
    }
    if !transaction.recovery_requested || transaction.boot != host.boot_id()? {
        fs::pointer(&store.code(), "active", &transaction.from.digest)?;
        host.marker("restart")?;
        let transaction = tx_mut(state)?;
        transaction.recovery_requested = true;
        transaction.boot = host.boot_id()?;
        transaction.deadline_ms = host.boottime_ms()? + 60000;
        store.save(state)?;
    } else if host.boottime_ms()? > transaction.deadline_ms {
        return Err(UpdateError::Recovery("rollback health not confirmed"));
    }
    Ok(())
}

async fn trial(
    host: &impl Host,
    store: &Store,
    state: &mut Journal,
    policy: &Policy,
) -> Result<()> {
    if tx(state)?.boot != host.boot_id()? {
        return rollback(host, store, state).await;
    }
    let process = host.properties(AGENT).await?;
    let transaction = tx_mut(state)?;
    let evidence = host.health(store, &process, &transaction.to.digest)?;
    if let Some(evidence) = evidence.filter(|e| e.invocation != transaction.old_invocation) {
        if transaction.stable_invocation.as_ref() != Some(&evidence.invocation) {
            transaction.stable_invocation = Some(evidence.invocation);
            transaction.stable_since_ms = Some(host.boottime_ms()?);
            transaction.stable_sample_ms = Some(evidence.collected_at_ms);
            store.save(state)?;
        } else if transaction.stable_since_ms.is_some_and(|s| {
            host.boottime_ms()
                .is_ok_and(|now| now.saturating_sub(s) >= policy.stability_seconds * 1000)
        }) && transaction.stable_sample_ms.is_some_and(|s| {
            evidence.collected_at_ms.saturating_sub(s) >= policy.stability_seconds * 1000
        }) {
            state.installed = transaction.to.clone();
            state.phase = Phase::Committed;
            store.save(state)?;
        }
    } else if transaction.stable_since_ms.is_some() {
        transaction.stable_since_ms = None;
        transaction.stable_invocation = None;
        transaction.stable_sample_ms = None;
        store.save(state)?;
    }
    if state.phase == Phase::Trial && host.boottime_ms()? > tx(state)?.deadline_ms {
        rollback(host, store, state).await?;
    }
    Ok(())
}

pub(crate) async fn reconcile(
    host: &impl Host,
    store: &Store,
    state: &mut Journal,
    policy: &Policy,
) -> Result<()> {
    if state.phase == Phase::ManualRecovery {
        return Err(UpdateError::Recovery("operator recovery required"));
    }
    if state.phase.terminal() {
        if store.active()? != state.installed.digest {
            return Err(UpdateError::Recovery("committed active pointer changed"));
        }
        return Ok(());
    }
    let active = store.active()?;
    if active != tx(state)?.from.digest && active != tx(state)?.to.digest {
        return Err(UpdateError::Recovery("unexpected active object"));
    }
    match state.phase {
        Phase::Prepared => prepared(host, store, state).await,
        Phase::Stopping => stopping(host, store, state, policy).await,
        Phase::Trial => trial(host, store, state, policy).await,
        Phase::RollbackIntent => rollback(host, store, state).await,
        _ => Err(UpdateError::Recovery("unexpected transaction phase")),
    }
}

#[cfg(test)]
#[path = "activate_tests.rs"]
mod tests;
