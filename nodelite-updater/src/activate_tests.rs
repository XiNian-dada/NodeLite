//! Deterministic acceptance harness for activation and rollback fault paths (#402).
//!
//! `Lab` stands in for systemd: it consumes the fixed bridge markers, moves the
//! fixed units through their states and serves Agent health evidence. Every
//! `tick` is one fresh updater invocation that reloads the journal from disk,
//! so a simulated crash at a `fault::checkpoint` cut point loses exactly the
//! in-memory state a killed process would. Invariants are asserted after every
//! tick. The real-systemd KVM procedure is documented in the README.

use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet};

use nodelite_proto::update_health::UpdateHealth;

use super::{Host, prepare, reconcile};
use crate::error::{Result, UpdateError};
use crate::policy::Policy;
use crate::state::{Artifact, Journal, Phase, Store};
use crate::system::{AGENT, PREFLIGHT, STOP, value};

const ROOT: &[u8] = b"public-root";
const SIMULATED_CRASH: &str = "simulated crash";

struct Fake {
    now_ms: Cell<u64>,
    boot: RefCell<String>,
    units: RefCell<BTreeMap<&'static str, BTreeMap<String, String>>>,
    markers: RefCell<BTreeSet<String>>,
    crash_at: RefCell<Option<&'static str>>,
    unhealthy: RefCell<BTreeSet<String>>,
    lingering: Cell<bool>,
    invocations: Cell<u64>,
    exits: Cell<u64>,
}

impl Fake {
    fn new() -> Self {
        let fake = Self {
            now_ms: Cell::new(1_000_000),
            boot: RefCell::new("boot-a".into()),
            units: RefCell::new(BTreeMap::new()),
            markers: RefCell::new(BTreeSet::new()),
            crash_at: RefCell::new(None),
            unhealthy: RefCell::new(BTreeSet::new()),
            lingering: Cell::new(false),
            invocations: Cell::new(0),
            exits: Cell::new(0),
        };
        fake.set(
            PREFLIGHT,
            &[("ActiveState", "inactive"), ("Result", "success")],
        );
        fake.set(STOP, &[("ActiveState", "inactive"), ("Result", "success")]);
        fake.start_agent();
        fake
    }

    fn set(&self, unit: &'static str, pairs: &[(&str, &str)]) {
        let mut units = self.units.borrow_mut();
        let entry = units.entry(unit).or_default();
        for (key, value) in pairs {
            entry.insert((*key).into(), (*value).into());
        }
    }

    fn next_exit(&self) -> String {
        self.exits.set(self.exits.get() + 1);
        (self.exits.get() * 1000).to_string()
    }

    fn start_agent(&self) {
        self.invocations.set(self.invocations.get() + 1);
        let invocation = format!("{:032x}", self.invocations.get());
        let pid = (4000 + self.invocations.get()).to_string();
        self.set(
            AGENT,
            &[
                ("ActiveState", "active"),
                ("MainPID", &pid),
                ("InvocationID", &invocation),
            ],
        );
    }

    /// Applies what the fixed units would do for each pending bridge marker.
    fn run_systemd(&self) {
        let pending = std::mem::take(&mut *self.markers.borrow_mut());
        for marker in pending {
            match marker.as_str() {
                "preflight" => {
                    let exit = self.next_exit();
                    self.set(
                        PREFLIGHT,
                        &[
                            ("ActiveState", "active"),
                            ("Result", "success"),
                            ("ExecMainExitTimestampMonotonic", &exit),
                        ],
                    );
                }
                "stop" => {
                    let exit = self.next_exit();
                    self.set(AGENT, &[("ActiveState", "inactive"), ("MainPID", "0")]);
                    self.set(
                        STOP,
                        &[
                            ("ActiveState", "inactive"),
                            ("Result", "success"),
                            ("ExecMainExitTimestampMonotonic", &exit),
                        ],
                    );
                }
                "restart" => self.start_agent(),
                other => panic!("unexpected bridge marker {other}"),
            }
        }
    }

    fn reboot(&self) {
        let next = format!("{}-next", self.boot.borrow());
        *self.boot.borrow_mut() = next;
        self.markers.borrow_mut().clear();
        self.start_agent();
    }

    fn advance(&self, ms: u64) {
        self.now_ms.set(self.now_ms.get() + ms);
    }
}

impl Host for Fake {
    async fn properties(&self, unit: &str) -> Result<BTreeMap<String, String>> {
        let unit = [AGENT, PREFLIGHT, STOP]
            .into_iter()
            .find(|fixed| *fixed == unit)
            .ok_or(UpdateError::Policy("non-fixed service rejected"))?;
        Ok(self.units.borrow().get(unit).cloned().unwrap_or_default())
    }
    fn boot_id(&self) -> Result<String> {
        Ok(self.boot.borrow().clone())
    }
    fn boottime_ms(&self) -> Result<u64> {
        Ok(self.now_ms.get())
    }
    fn marker(&self, action: &str) -> Result<()> {
        self.markers.borrow_mut().insert(action.into());
        Ok(())
    }
    fn marker_pending(&self, name: &str) -> bool {
        self.markers.borrow().contains(name)
    }
    fn stopped(&self, properties: &BTreeMap<String, String>) -> Result<bool> {
        Ok(value(properties, "ActiveState") == "inactive"
            && value(properties, "MainPID") == "0"
            && !self.lingering.get())
    }
    fn health(
        &self,
        store: &Store,
        process: &BTreeMap<String, String>,
        expected: &str,
    ) -> Result<Option<UpdateHealth>> {
        // Mirrors system::health: evidence only counts for the active pointer.
        if value(process, "ActiveState") != "active"
            || store.active()? != expected
            || self.unhealthy.borrow().contains(expected)
        {
            return Ok(None);
        }
        Ok(Some(UpdateHealth {
            schema: 1,
            pid: value(process, "MainPID").parse().unwrap_or(1),
            invocation: value(process, "InvocationID").into(),
            executable_sha256: expected.into(),
            sample_boottime_ms: self.now_ms.get(),
            collected_at_ms: self.now_ms.get(),
            sample_counter: 1,
        }))
    }
    fn checkpoint(&self, name: &str) -> Result<()> {
        if self.crash_at.borrow().is_some_and(|point| point == name) {
            *self.crash_at.borrow_mut() = None;
            return Err(UpdateError::Recovery(SIMULATED_CRASH));
        }
        Ok(())
    }
}

struct Lab {
    _directory: tempfile::TempDir,
    store: Store,
    policy: Policy,
    fake: Fake,
    from: Artifact,
    to: Artifact,
}

impl Lab {
    fn new() -> Self {
        let directory = tempfile::tempdir().expect("temporary directory");
        let store = Store {
            root: directory.path().to_path_buf(),
        };
        for path in [store.private(), store.code(), store.code().join("objects")] {
            crate::fs::directory(&path, 0o700).expect("store directory");
        }
        let policy = crate::policy::fixture();
        let baseline = directory.path().join("baseline");
        std::fs::write(&baseline, b"baseline").expect("baseline");
        store
            .initialize(&policy, ROOT, &baseline)
            .expect("initialize");
        let candidate = b"candidate";
        let digest = crate::fs::digest(candidate);
        crate::fs::atomic(
            &store.object(&digest).expect("object path"),
            candidate,
            0o550,
        )
        .expect("candidate object");
        let from = Artifact {
            digest: policy.baseline_sha256.clone(),
            sequence: policy.baseline_sequence,
        };
        let to = Artifact {
            digest,
            sequence: policy.baseline_sequence + 1,
        };
        // A verified download records the candidate as seen before activation.
        let mut journal = store.load(&policy, ROOT).expect("journal");
        journal.seen_sequence = to.sequence;
        journal.seen_digest = to.digest.clone();
        store.save(&journal).expect("seen candidate");
        Self {
            _directory: directory,
            store,
            policy,
            fake: Fake::new(),
            from,
            to,
        }
    }

    fn journal(&self) -> Journal {
        self.store
            .load(&self.policy, ROOT)
            .expect("journal reloads")
    }

    fn active(&self) -> String {
        self.store.active().expect("active pointer")
    }

    fn assert_invariants(&self) {
        let journal = self.journal();
        let active = self.active();
        if journal.phase.terminal() {
            assert_eq!(active, journal.installed.digest, "terminal pointer drift");
        } else if journal.phase != Phase::ManualRecovery {
            assert!(
                active == self.from.digest || active == self.to.digest,
                "active pointer left the transaction"
            );
        }
        if journal.phase == Phase::Committed {
            assert_eq!(journal.installed, self.to);
        }
        if journal.phase == Phase::RolledBack {
            assert_eq!(journal.installed, self.from);
        }
    }

    async fn start(&self) -> Result<()> {
        let mut journal = self.journal();
        let outcome = prepare(
            &self.fake,
            &self.store,
            &mut journal,
            &self.policy,
            self.to.clone(),
        )
        .await;
        self.settle(outcome)
    }

    /// One updater invocation followed by systemd reacting to its markers.
    async fn tick(&self) -> Result<()> {
        let mut journal = self.journal();
        let outcome = reconcile(&self.fake, &self.store, &mut journal, &self.policy).await;
        if let Err(error) = &outcome
            && !is_crash(error)
        {
            crate::runtime::latch_failure(&self.store, &mut journal, error);
        }
        self.settle(outcome)
    }

    fn settle(&self, outcome: Result<()>) -> Result<()> {
        self.fake.run_systemd();
        self.assert_invariants();
        match outcome {
            Err(error) if is_crash(&error) => Ok(()),
            other => other,
        }
    }

    /// Ticks one second at a time until the journal reaches a terminal phase.
    async fn run_to_terminal(&self) -> Phase {
        for _ in 0..200 {
            self.tick().await.expect("reconcile");
            let phase = self.journal().phase;
            if phase.terminal() {
                return phase;
            }
            self.fake.advance(1000);
        }
        panic!("transaction did not settle: {:?}", self.journal().phase);
    }

    fn crash_at(&self, point: &'static str) {
        *self.fake.crash_at.borrow_mut() = Some(point);
    }

    fn assert_rolled_back(&self) {
        let journal = self.journal();
        assert_eq!(journal.phase, Phase::RolledBack);
        assert_eq!(self.active(), self.from.digest);
        assert!(journal.failed.contains(&self.to.digest));
    }

    fn assert_manual_recovery(&self) {
        assert_eq!(self.journal().phase, Phase::ManualRecovery);
        assert!(self.store.private().join("recovery-required.json").exists());
    }
}

fn is_crash(error: &UpdateError) -> bool {
    matches!(error, UpdateError::Recovery(SIMULATED_CRASH))
}

#[tokio::test(flavor = "current_thread")]
async fn healthy_candidate_commits_through_every_phase() {
    let lab = Lab::new();
    lab.start().await.expect("prepare");
    assert_eq!(lab.journal().phase, Phase::Prepared);
    lab.tick().await.expect("preflight confirmed");
    assert_eq!(lab.journal().phase, Phase::Stopping);
    lab.tick().await.expect("stop confirmed");
    assert_eq!(lab.journal().phase, Phase::Trial);
    assert_eq!(lab.active(), lab.to.digest);
    assert_eq!(lab.run_to_terminal().await, Phase::Committed);
    assert_eq!(lab.active(), lab.to.digest);
}

#[tokio::test(flavor = "current_thread")]
async fn crash_before_the_stop_barrier_still_commits() {
    for point in ["prepared", "stopping"] {
        let lab = Lab::new();
        lab.crash_at(point);
        lab.start().await.expect("prepare");
        assert_eq!(lab.run_to_terminal().await, Phase::Committed, "{point}");
        assert_eq!(lab.active(), lab.to.digest, "{point}");
    }
}

#[tokio::test(flavor = "current_thread")]
async fn crash_around_the_pointer_flip_rolls_back_to_the_old_object() {
    // Neither cut point leaves a restart request behind, so the candidate never
    // proves health and the trial deadline must restore the old object.
    for point in ["trial_journal", "active_pointer"] {
        let lab = Lab::new();
        lab.crash_at(point);
        lab.start().await.expect("prepare");
        lab.tick().await.expect("preflight confirmed");
        lab.tick().await.expect("stop confirmed");
        let journal = lab.journal();
        assert_eq!(journal.phase, Phase::Trial, "{point}");
        let expected = if point == "trial_journal" {
            &lab.from.digest
        } else {
            &lab.to.digest
        };
        assert_eq!(&lab.active(), expected, "{point}");
        assert_eq!(lab.run_to_terminal().await, Phase::RolledBack, "{point}");
        lab.assert_rolled_back();
    }
}

#[tokio::test(flavor = "current_thread")]
async fn failed_health_confirmation_rolls_back_once() {
    let lab = Lab::new();
    lab.fake
        .unhealthy
        .borrow_mut()
        .insert(lab.to.digest.clone());
    lab.start().await.expect("prepare");
    assert_eq!(lab.run_to_terminal().await, Phase::RolledBack);
    lab.assert_rolled_back();
    let journal = lab.journal();
    let transaction = journal.transaction.as_ref().expect("transaction kept");
    assert!(transaction.rollback_consumed);
    assert!(transaction.recovery_requested);
}

#[tokio::test(flavor = "current_thread")]
async fn crash_after_rollback_intent_resumes_the_rollback() {
    let lab = Lab::new();
    lab.fake
        .unhealthy
        .borrow_mut()
        .insert(lab.to.digest.clone());
    lab.crash_at("rollback_intent");
    lab.start().await.expect("prepare");
    for _ in 0..200 {
        lab.tick().await.expect("reconcile");
        if lab.journal().phase == Phase::RollbackIntent {
            break;
        }
        lab.fake.advance(1000);
    }
    assert_eq!(lab.journal().phase, Phase::RollbackIntent);
    assert!(
        lab.fake.crash_at.borrow().is_none(),
        "cut point was reached"
    );
    assert_eq!(lab.run_to_terminal().await, Phase::RolledBack);
    lab.assert_rolled_back();
}

#[tokio::test(flavor = "current_thread")]
async fn unconfirmed_rollback_latches_operator_recovery() {
    let lab = Lab::new();
    lab.fake
        .unhealthy
        .borrow_mut()
        .insert(lab.to.digest.clone());
    lab.fake
        .unhealthy
        .borrow_mut()
        .insert(lab.from.digest.clone());
    lab.start().await.expect("prepare");
    let mut failure = None;
    for _ in 0..300 {
        if let Err(error) = lab.tick().await {
            failure = Some(error);
            break;
        }
        lab.fake.advance(1000);
    }
    assert!(matches!(
        failure,
        Some(UpdateError::Recovery("rollback health not confirmed"))
    ));
    lab.assert_manual_recovery();
    assert!(matches!(
        lab.tick().await,
        Err(UpdateError::Recovery("operator recovery required"))
    ));
}

#[tokio::test(flavor = "current_thread")]
async fn old_service_that_never_exits_blocks_the_flip_then_latches() {
    let lab = Lab::new();
    lab.fake.lingering.set(true);
    lab.start().await.expect("prepare");
    lab.tick().await.expect("preflight confirmed");
    let mut failure = None;
    for _ in 0..60 {
        assert_eq!(lab.active(), lab.from.digest, "pointer flipped too early");
        if let Err(error) = lab.tick().await {
            failure = Some(error);
            break;
        }
        assert_eq!(lab.journal().phase, Phase::Stopping);
        lab.fake.advance(1000);
    }
    assert!(matches!(
        failure,
        Some(UpdateError::Recovery("old service stop was not confirmed"))
    ));
    lab.assert_manual_recovery();
    assert_eq!(lab.active(), lab.from.digest);
}

#[tokio::test(flavor = "current_thread")]
async fn reboot_before_the_flip_abandons_the_candidate() {
    for phase in [Phase::Prepared, Phase::Stopping] {
        let lab = Lab::new();
        lab.start().await.expect("prepare");
        if phase == Phase::Stopping {
            lab.tick().await.expect("preflight confirmed");
        }
        assert_eq!(lab.journal().phase, phase);
        lab.fake.reboot();
        lab.tick().await.expect("reconcile after reboot");
        assert_eq!(lab.journal().phase, Phase::RolledBack, "{phase:?}");
        assert_eq!(lab.active(), lab.from.digest, "{phase:?}");
    }
}

#[tokio::test(flavor = "current_thread")]
async fn reboot_during_trial_rolls_back() {
    let lab = Lab::new();
    lab.start().await.expect("prepare");
    lab.tick().await.expect("preflight confirmed");
    lab.tick().await.expect("stop confirmed");
    assert_eq!(lab.journal().phase, Phase::Trial);
    lab.fake.reboot();
    assert_eq!(lab.run_to_terminal().await, Phase::RolledBack);
    lab.assert_rolled_back();
}

#[tokio::test(flavor = "current_thread")]
async fn prepare_refuses_a_second_transaction_while_one_is_open() {
    let lab = Lab::new();
    lab.start().await.expect("prepare");
    assert!(matches!(lab.start().await, Err(UpdateError::Policy(_))));
    assert_eq!(lab.journal().phase, Phase::Prepared);
}
