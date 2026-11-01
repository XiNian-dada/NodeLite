//! Fixed local operations; remote signals never select commands, URLs or privileged actions.

use std::path::{Path, PathBuf};

use clap::{Parser, Subcommand};

use crate::activate;
use crate::clock::FloorReset;
use crate::download;
use crate::error::{Result, UpdateError};
use crate::fs::Lock;
use crate::gate::{self, Facts, Gate};
use crate::policy::{Policy, ROOT, trusted_file};
use crate::state::{Journal, Store};
use crate::system;

#[derive(Parser)]
#[command(
    name = "nodelite-updater",
    about = "Opt-in Linux/systemd signed Agent updater"
)]
struct Cli {
    #[command(subcommand)]
    action: Action,
}

#[derive(Subcommand)]
enum Action {
    /// Explicitly enroll the root-policy-pinned baseline; refuses existing state.
    Initialize {
        #[arg(long)]
        baseline: PathBuf,
    },
    /// Verify a first enrollment against signed metadata without changing any service.
    VerifyEnrollment {
        #[arg(long)]
        baseline: PathBuf,
    },
    /// Verify and download from the fixed repository without stopping the Agent.
    Download,
    /// Download and request a journaled activation through fixed systemd units.
    Apply,
    /// Consume one bounded local request, only when root explicitly allows remote triggers.
    Remote,
    /// Resume a pending transaction; does not fetch a new release.
    Reconcile {
        #[arg(long)]
        boot: bool,
    },
    Status,
    /// Lower the local time floor to now after a forward clock jump; refuses open transactions.
    ResetTimeFloor {
        #[arg(long)]
        confirm_clock_correct: bool,
    },
}

/// Runs only as the dedicated updater UID after explicit local provisioning.
pub async fn run() -> Result<()> {
    // The reference library's working files must remain private even for manual CLI runs.
    // SAFETY: umask takes a scalar; the CLI intentionally sets process-wide creation policy here.
    unsafe {
        libc::umask(0o077);
    }
    let cli = Cli::parse();
    let _ = rustls::crypto::ring::default_provider().install_default();
    let policy = Policy::read()?;
    // Pausing stops new releases only; journal recovery must still run under the same checks.
    if matches!(
        cli.action,
        Action::Reconcile { .. } | Action::Status | Action::ResetTimeFloor { .. }
    ) {
        policy.validate_paused()?;
    } else {
        policy.validate()?;
    }
    let bootstrap = trusted_file(Path::new(ROOT), 262144)?;
    system::guard(!matches!(&cli.action, Action::VerifyEnrollment { .. })).await?;
    let store = Store::system();
    system::paths_safe(&store)?;
    if matches!(cli.action, Action::Remote) {
        let (request, _lock) = crate::remote::take(&store).await?;
        return crate::remote::run(&store, &policy, &bootstrap, request).await;
    }
    let _lock = Lock::acquire(&store.private().join("lock"))?;
    if let Action::VerifyEnrollment { baseline } = &cli.action {
        crate::enrollment::verify(&store, &policy, &bootstrap, baseline).await?;
        println!("signed enrollment verified without starting or changing any service");
        return Ok(());
    }
    if let Action::Initialize { baseline } = cli.action {
        store.initialize(&policy, &bootstrap, &baseline)?;
        println!("enrolled baseline without starting or changing any service");
        return Ok(());
    }
    dispatch(cli.action, &store, &policy, &bootstrap).await
}

async fn dispatch(action: Action, store: &Store, policy: &Policy, bootstrap: &[u8]) -> Result<()> {
    let mut state = match store.load(policy, bootstrap) {
        Ok(state) => state,
        Err(error) => {
            let _ = system::manual(store, "journal or committed object invalid");
            return Err(error);
        }
    };
    if let Action::Reconcile { boot } = action
        && let Gate::Idle(reason) = idle(store, &state, policy)?
    {
        if boot {
            // Markers from the previous boot must not replay against an idle Agent.
            system::clear_markers()?;
        }
        if state.phase.terminal() {
            remove_pending(store)?;
        }
        println!("{reason}");
        return Ok(());
    }
    if !matches!(action, Action::Status) && store.private().join("recovery-required.json").exists()
    {
        return Err(UpdateError::Recovery("operator recovery required"));
    }
    // Returns before the failure path below, which would latch manual recovery on a mere refusal.
    if let Action::ResetTimeFloor {
        confirm_clock_correct,
    } = action
    {
        return reset_time_floor(store, &mut state, confirm_clock_correct);
    }
    let result = match action {
        Action::Status => Ok(()),
        Action::Reconcile { boot } => {
            if boot {
                system::clear_markers()?;
            }
            crate::remote::recover_download(store, &mut state)?;
            activate::reconcile(&activate::SystemHost, store, &mut state, policy).await
        }
        Action::Download | Action::Apply => {
            if !state.phase.terminal() {
                return Err(UpdateError::Busy);
            }
            crate::maintenance::collect(store, &state)?;
            let candidate = download::fetch(store, &mut state, policy, bootstrap).await?;
            if matches!(action, Action::Apply) && candidate != state.installed {
                activate::prepare(&activate::SystemHost, store, &mut state, policy, candidate).await
            } else {
                if matches!(action, Action::Apply) {
                    crate::remote::up_to_date(&mut state);
                    store.save(&state)?;
                }
                Ok(())
            }
        }
        Action::Initialize { .. }
        | Action::VerifyEnrollment { .. }
        | Action::Remote
        | Action::ResetTimeFloor { .. } => {
            Err(UpdateError::Recovery("initialization dispatch failed"))
        }
    };
    if let Err(error) = result {
        latch_failure(store, &mut state, &error);
        return Err(error);
    }
    if state.phase.terminal() {
        remove_pending(store)?;
    }
    println!(
        "{}",
        serde_json::to_string(&serde_json::json!({
            "phase": state.phase, "installed_sequence": state.installed.sequence,
            "installed_sha256": state.installed.digest, "accepted_sequence": state.seen_sequence,
            "manual_recovery": store.private().join("recovery-required.json").exists(),
        }))?
    );
    Ok(())
}

/// Latches operator recovery when a failure leaves an unfinished transaction.
pub(crate) fn latch_failure(store: &Store, state: &mut Journal, error: &UpdateError) {
    if !state.phase.terminal() || matches!(error, UpdateError::Recovery(_)) {
        state.phase = crate::state::Phase::ManualRecovery;
        let _ = store.save(state);
        let _ = system::manual(store, "activation could not be safely reconciled");
    }
}

fn idle(store: &Store, state: &Journal, policy: &Policy) -> Result<Gate> {
    let recovery_required = store.private().join("recovery-required.json").exists();
    if !recovery_required && policy.enabled {
        return Ok(Gate::Proceed);
    }
    let active = store.active()?;
    let transaction = state.transaction.as_ref();
    // A revoked rollback target is not known-good even though its digest still verifies.
    let from = transaction
        .filter(|tx| tx.from.sequence >= state.hard_floor)
        .map(|tx| tx.from.digest.as_str());
    Ok(gate::reconcile(Facts {
        paused: !policy.enabled,
        recovery_required,
        terminal: state.phase.terminal(),
        active: &active,
        installed: &state.installed.digest,
        from: from.filter(|_| !state.phase.terminal()),
    }))
}

fn reset_time_floor(store: &Store, state: &mut Journal, confirmed: bool) -> Result<()> {
    match store.reset_time_floor(state, confirmed, crate::state::epoch()?)? {
        FloorReset::Unchanged => println!(
            "time floor {} is not ahead of the clock; nothing to reset",
            state.time_high
        ),
        FloorReset::Lowered { old, new } => println!("time floor lowered from {old} to {new}"),
    }
    Ok(())
}

fn remove_pending(store: &Store) -> Result<()> {
    match std::fs::remove_file(store.private().join("pending")) {
        Ok(()) => crate::fs::sync_dir(&store.private()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

pub(crate) async fn apply(store: &Store, policy: &Policy, bootstrap: &[u8]) -> Result<()> {
    dispatch(Action::Apply, store, policy, bootstrap).await
}
