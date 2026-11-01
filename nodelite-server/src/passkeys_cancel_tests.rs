//! Cancellation consistency checks for passkey commits (#376).
//!
//! The caller task is aborted while the first blocking write is parked, which
//! models a dropped HTTP handler on a slow disk; another mutation then queues
//! behind it. Ordering is driven by the commit observer, not by sleeps.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use uuid::Uuid;
use webauthn_rs::prelude::{AuthenticationResult, Passkey};

use super::{
    CommitObserver, PasskeyCeremonies, PasskeyError, PasskeyFile, PasskeyService, StoredPasskey,
    passkey_storage_path,
};

const WAIT: Duration = Duration::from_secs(10);

#[derive(Default)]
struct Counters {
    contended: usize,
    parked: bool,
    released: bool,
    in_flight: usize,
    max_in_flight: usize,
    finished: usize,
}

/// Parks the first write until released and records write overlap.
#[derive(Default)]
struct Probe {
    counters: Mutex<Counters>,
    changed: Condvar,
}

impl Probe {
    fn update(&self, apply: impl FnOnce(&mut Counters)) {
        apply(&mut self.counters.lock().expect("probe lock"));
        self.changed.notify_all();
    }

    async fn wait_for(self: &Arc<Self>, ready: impl Fn(&Counters) -> bool + Send + 'static) {
        let probe = Arc::clone(self);
        tokio::task::spawn_blocking(move || {
            let counters = probe.counters.lock().expect("probe lock");
            let (_counters, timeout) = probe
                .changed
                .wait_timeout_while(counters, WAIT, |counters| !ready(counters))
                .expect("probe lock");
            assert!(!timeout.timed_out(), "probe condition not reached");
        })
        .await
        .expect("probe wait task");
    }

    fn release(&self) {
        self.update(|counters| counters.released = true);
    }

    fn max_in_flight(&self) -> usize {
        self.counters.lock().expect("probe lock").max_in_flight
    }
}

impl CommitObserver for Probe {
    fn contended(&self) {
        self.update(|counters| counters.contended += 1);
    }

    fn before_write(&self) {
        let mut counters = self.counters.lock().expect("probe lock");
        counters.in_flight += 1;
        counters.max_in_flight = counters.max_in_flight.max(counters.in_flight);
        self.changed.notify_all();
        if !counters.parked {
            counters.parked = true;
            self.changed.notify_all();
            let (_counters, timeout) = self
                .changed
                .wait_timeout_while(counters, WAIT, |counters| !counters.released)
                .expect("probe lock");
            assert!(!timeout.timed_out(), "parked write was never released");
        }
    }

    fn after_write(&self) {
        self.update(|counters| counters.in_flight -= 1);
    }

    fn finished(&self) {
        self.update(|counters| counters.finished += 1);
    }
}

fn fake_passkey(cred_id: &str) -> Passkey {
    let json = format!(
        r#"{{"cred":{{"cred_id":"{cred_id}","cred":{{"type_":"ES256","key":{{"EC_EC2":{{"curve":"SECP256R1","x":[1],"y":[2]}}}}}},"counter":0,"transports":null,"user_verified":true,"backup_eligible":false,"backup_state":false,"registration_policy":"required","extensions":{{"cred_protect":"NotRequested","hmac_create_secret":"NotRequested"}},"attestation":{{"data":"None","metadata":"None"}},"attestation_format":"none"}}}}"#
    );
    serde_json::from_str(&json).expect("fake passkey fixture parses")
}

fn authentication(cred_id: &str, counter: u32) -> AuthenticationResult {
    let json = format!(
        r#"{{"cred_id":"{cred_id}","needs_update":true,"user_verified":true,"backup_state":false,"backup_eligible":false,"counter":{counter},"extensions":{{}}}}"#
    );
    serde_json::from_str(&json).expect("authentication fixture parses")
}

fn stored(cred_id: &str) -> StoredPasskey {
    StoredPasskey {
        id: Uuid::new_v4(),
        label: cred_id.to_string(),
        created_at: chrono::Utc::now(),
        passkey: fake_passkey(cred_id),
    }
}

struct Fixture {
    dir: PathBuf,
    service: PasskeyService,
    probe: Arc<Probe>,
    ids: Vec<Uuid>,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

impl Fixture {
    async fn new(cred_ids: &[&str]) -> Self {
        let dir = std::env::temp_dir().join(format!("nodelite-passkey-cancel-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let credentials = PasskeyFile {
            account_id: Uuid::new_v4(),
            credentials: cred_ids.iter().map(|id| stored(id)).collect(),
        };
        let ids = credentials.credentials.iter().map(|c| c.id).collect();
        let mut service = PasskeyService {
            webauthn: None,
            path: Arc::new(passkey_storage_path(&dir.join("server.toml"))),
            credentials: Arc::new(Mutex::new(credentials.clone())),
            ceremonies: Arc::new(Mutex::new(PasskeyCeremonies {
                registrations: HashMap::new(),
                authentications: HashMap::new(),
            })),
            mutation_lock: Arc::new(tokio::sync::Mutex::new(())),
            observer: None,
        };
        service.persist(&credentials).await.expect("seed passkeys");
        let probe = Arc::new(Probe::default());
        service.observer = Some(probe.clone());
        Self {
            dir,
            service,
            probe,
            ids,
        }
    }

    fn memory(&self) -> serde_json::Value {
        let credentials = self.service.credentials_snapshot().expect("snapshot");
        serde_json::to_value(&credentials).expect("memory json")
    }

    fn disk(&self) -> serde_json::Value {
        let bytes = std::fs::read(self.service.path.as_ref()).expect("read passkeys");
        serde_json::from_slice(&bytes).expect("disk json")
    }

    /// Waits for `count` commit tasks, then checks disk, memory and a reload agree.
    async fn settled(&self, count: usize) -> Vec<Uuid> {
        self.probe
            .wait_for(move |counters| counters.finished >= count)
            .await;
        assert_eq!(self.probe.max_in_flight(), 1, "passkey writes overlapped");
        assert_eq!(self.memory(), self.disk(), "memory and disk diverged");
        let reloaded = reload(&self.dir).await;
        assert_eq!(reloaded, ids_of(&self.service), "reload diverged");
        reloaded
    }

    async fn park_first_write(&self) {
        self.probe.wait_for(|counters| counters.parked).await;
    }

    /// Releases the parked write once another commit is queued on the lock.
    async fn release_after_contention(&self) {
        self.probe
            .wait_for(|counters| counters.contended >= 1)
            .await;
        self.probe.release();
    }
}

fn ids_of(service: &PasskeyService) -> Vec<Uuid> {
    service
        .summaries()
        .expect("summaries")
        .into_iter()
        .map(|summary| summary.id)
        .collect()
}

async fn reload(dir: &Path) -> Vec<Uuid> {
    let service = PasskeyService::load(&dir.join("server.toml"), "https://monitor.example.com")
        .await
        .expect("reload passkeys");
    ids_of(&service)
}

async fn abort<T>(caller: tokio::task::JoinHandle<Result<T, PasskeyError>>) {
    caller.abort();
    assert!(caller.await.is_err_and(|error| error.is_cancelled()));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancelled_delete_mid_write_is_not_undone_by_next_delete() {
    let fixture = Fixture::new(&["AAAA", "AQEB", "AgIC"]).await;
    let (revoked, second, kept) = (fixture.ids[0], fixture.ids[1], fixture.ids[2]);

    let service = fixture.service.clone();
    let caller = tokio::spawn(async move { service.delete(revoked).await });
    fixture.park_first_write().await;
    abort(caller).await;

    let service = fixture.service.clone();
    let follow_up = tokio::spawn(async move { service.delete(second).await });
    fixture.release_after_contention().await;
    follow_up
        .await
        .expect("follow-up task")
        .expect("follow-up delete");

    assert_eq!(fixture.settled(2).await, vec![kept]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancelled_registration_mid_write_publishes_to_memory_and_disk() {
    let fixture = Fixture::new(&["AAAA"]).await;
    let existing = fixture.ids[0];

    let service = fixture.service.clone();
    let caller = tokio::spawn(async move {
        service
            .add_credential("laptop".to_string(), fake_passkey("AwMD"), || true)
            .await
    });
    fixture.park_first_write().await;
    abort(caller).await;

    let service = fixture.service.clone();
    let follow_up = tokio::spawn(async move { service.delete(existing).await });
    fixture.release_after_contention().await;
    follow_up
        .await
        .expect("follow-up task")
        .expect("follow-up delete");

    let remaining = fixture.settled(2).await;
    assert_eq!(remaining.len(), 1);
    assert_ne!(remaining[0], existing);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancelled_authentication_keeps_the_sign_counter_on_disk_and_in_memory() {
    let fixture = Fixture::new(&["AAAA", "AQEB"]).await;
    let other = fixture.ids[1];

    let service = fixture.service.clone();
    let caller = tokio::spawn(async move {
        service
            .record_authentication(authentication("AAAA", 5))
            .await
    });
    fixture.park_first_write().await;
    abort(caller).await;

    let service = fixture.service.clone();
    let follow_up = tokio::spawn(async move { service.delete(other).await });
    fixture.release_after_contention().await;
    follow_up
        .await
        .expect("follow-up task")
        .expect("follow-up delete");

    assert_eq!(fixture.settled(2).await, vec![fixture.ids[0]]);
    let counter = &fixture.disk()["credentials"][0]["passkey"]["cred"]["counter"];
    assert_eq!(counter, 5, "sign counter regressed");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancelled_while_waiting_for_lock_still_commits_consistently() {
    let fixture = Fixture::new(&["AAAA", "AQEB"]).await;
    let (first, queued) = (fixture.ids[0], fixture.ids[1]);

    let service = fixture.service.clone();
    let holder = tokio::spawn(async move { service.delete(first).await });
    fixture.park_first_write().await;

    let waiter = tokio::time::timeout(Duration::ZERO, fixture.service.delete(queued)).await;
    assert!(
        waiter.is_err(),
        "queued delete is still waiting for the lock"
    );

    fixture.release_after_contention().await;
    holder.await.expect("holder task").expect("holder delete");

    assert_eq!(fixture.settled(2).await, Vec::<Uuid>::new());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn queued_registration_is_rejected_once_its_session_is_revoked() {
    let fixture = Fixture::new(&["AAAA", "AQEB"]).await;
    let (first, kept) = (fixture.ids[0], fixture.ids[1]);
    let confirmed = Arc::new(AtomicBool::new(true));

    let service = fixture.service.clone();
    let holder = tokio::spawn(async move { service.delete(first).await });
    fixture.park_first_write().await;

    let session = Arc::clone(&confirmed);
    let registration =
        fixture
            .service
            .add_credential("laptop".to_string(), fake_passkey("AwMD"), move || {
                session.load(Ordering::SeqCst)
            });
    let waiter = tokio::time::timeout(Duration::ZERO, registration).await;
    assert!(waiter.is_err(), "queued registration is still waiting");

    confirmed.store(false, Ordering::SeqCst);
    fixture.release_after_contention().await;
    holder.await.expect("holder task").expect("holder delete");

    assert_eq!(fixture.settled(2).await, vec![kept]);
}
