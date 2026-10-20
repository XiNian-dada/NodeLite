//! Local administration only: no remote commands, arbitrary download URLs or root execution.

use std::path::{Path, PathBuf};

use clap::{Parser, Subcommand};

use crate::activate;
use crate::download;
use crate::error::{Result, UpdateError};
use crate::fs::Lock;
use crate::policy::{Policy, ROOT, trusted_file};
use crate::state::Store;
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
    /// Resume a pending transaction; does not fetch a new release.
    Reconcile {
        #[arg(long)]
        boot: bool,
    },
    Status,
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
    let policy = Policy::load()?;
    let bootstrap = trusted_file(Path::new(ROOT), 262144)?;
    system::guard(!matches!(&cli.action, Action::VerifyEnrollment { .. })).await?;
    let store = Store::system();
    system::paths_safe(&store)?;
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
    if !matches!(action, Action::Status) && store.private().join("recovery-required.json").exists()
    {
        return Err(UpdateError::Recovery("operator recovery required"));
    }
    let result = match action {
        Action::Status => Ok(()),
        Action::Reconcile { boot } => {
            if boot {
                system::clear_markers()?;
            }
            activate::reconcile(store, &mut state, policy).await
        }
        Action::Download | Action::Apply => {
            if !state.phase.terminal() {
                return Err(UpdateError::Busy);
            }
            crate::maintenance::collect(store, &state)?;
            let candidate = download::fetch(store, &mut state, policy, bootstrap).await?;
            if matches!(action, Action::Apply) && candidate != state.installed {
                activate::prepare(store, &mut state, policy, candidate).await
            } else {
                Ok(())
            }
        }
        Action::Initialize { .. } | Action::VerifyEnrollment { .. } => {
            Err(UpdateError::Recovery("initialization dispatch failed"))
        }
    };
    if let Err(error) = result {
        if !state.phase.terminal() || matches!(error, UpdateError::Recovery(_)) {
            state.phase = crate::state::Phase::ManualRecovery;
            let _ = store.save(&state);
            let _ = system::manual(store, "activation could not be safely reconciled");
        }
        return Err(error);
    }
    if state.phase.terminal() {
        match std::fs::remove_file(store.private().join("pending")) {
            Ok(()) => crate::fs::sync_dir(&store.private())?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
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
