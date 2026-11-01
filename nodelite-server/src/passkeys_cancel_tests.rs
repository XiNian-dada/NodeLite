//! Cancellation consistency checks for passkey commits (#376).
//!
//! The caller task is aborted while the blocking write is parked, which models
//! a dropped HTTP handler on a slow disk; a second mutation then follows it.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

use uuid::Uuid;
use webauthn_rs::prelude::Passkey;

use super::{PasskeyCeremonies, PasskeyFile, PasskeyService, StoredPasskey, passkey_storage_path};

struct WriteGate {
    reached: Mutex<mpsc::Receiver<()>>,
    release: mpsc::Sender<()>,
}

type WriteHook = Arc<dyn Fn() + Send + Sync>;

impl WriteGate {
    /// Parks only the first blocking write so later mutations run freely.
    fn install() -> (Arc<Self>, WriteHook) {
        let (reached_tx, reached_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel::<()>();
        let release_rx = Mutex::new(release_rx);
        let first = AtomicBool::new(true);
        let hook = move || {
            if first.swap(false, Ordering::SeqCst) {
                reached_tx.send(()).expect("test observes the parked write");
                release_rx
                    .lock()
                    .expect("release receiver lock")
                    .recv_timeout(Duration::from_secs(10))
                    .expect("test releases the parked write");
            }
        };
        let gate = Self {
            reached: Mutex::new(reached_rx),
            release: release_tx,
        };
        (Arc::new(gate), Arc::new(hook))
    }

    async fn wait_reached(self: &Arc<Self>) {
        let gate = Arc::clone(self);
        tokio::task::spawn_blocking(move || {
            gate.reached
                .lock()
                .expect("reached receiver lock")
                .recv_timeout(Duration::from_secs(10))
                .expect("first write reaches the gate");
        })
        .await
        .expect("gate wait task");
    }

    fn release(&self) {
        self.release
            .send(())
            .expect("parked write is still waiting");
    }
}

fn fake_passkey(cred_id: &str) -> Passkey {
    let json = format!(
        r#"{{"cred":{{"cred_id":"{cred_id}","cred":{{"type_":"ES256","key":{{"EC_EC2":{{"curve":"SECP256R1","x":[1],"y":[2]}}}}}},"counter":0,"transports":null,"user_verified":true,"backup_eligible":false,"backup_state":false,"registration_policy":"required","extensions":{{"cred_protect":"NotRequested","hmac_create_secret":"NotRequested"}},"attestation":{{"data":"None","metadata":"None"}},"attestation_format":"none"}}}}"#
    );
    serde_json::from_str(&json).expect("fake passkey fixture parses")
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
    ids: Vec<Uuid>,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

impl Fixture {
    async fn new(cred_ids: &[&str], hook: WriteHook) -> Self {
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
            before_write: None,
        };
        service.persist(&credentials).await.expect("seed passkeys");
        service.before_write = Some(hook);
        Self { dir, service, ids }
    }

    fn memory_ids(&self) -> Vec<Uuid> {
        ids_of(&self.service)
    }

    async fn disk_ids(&self) -> Vec<Uuid> {
        reload(&self.dir).await
    }

    async fn settle(&self) {
        // Taking the lock proves every detached commit has published.
        drop(self.service.mutation_lock.lock().await);
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancelled_delete_mid_write_is_not_undone_by_next_delete() {
    let (gate, hook) = WriteGate::install();
    let fixture = Fixture::new(&["AAAA", "AQEB", "AgIC"], hook).await;
    let (revoked, second, kept) = (fixture.ids[0], fixture.ids[1], fixture.ids[2]);

    let service = fixture.service.clone();
    let caller = tokio::spawn(async move { service.delete(revoked).await });
    gate.wait_reached().await;
    caller.abort();
    assert!(caller.await.is_err_and(|error| error.is_cancelled()));

    let service = fixture.service.clone();
    let follow_up = tokio::spawn(async move { service.delete(second).await });
    tokio::time::sleep(Duration::from_millis(50)).await;
    gate.release();
    follow_up
        .await
        .expect("follow-up task")
        .expect("follow-up delete");
    fixture.settle().await;

    assert_eq!(fixture.memory_ids(), vec![kept]);
    assert_eq!(fixture.disk_ids().await, vec![kept]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancelled_registration_mid_write_publishes_to_memory_and_disk() {
    let (gate, hook) = WriteGate::install();
    let fixture = Fixture::new(&["AAAA"], hook).await;
    let existing = fixture.ids[0];

    let service = fixture.service.clone();
    let caller = tokio::spawn(async move {
        service
            .add_credential("laptop".to_string(), fake_passkey("AwMD"))
            .await
    });
    gate.wait_reached().await;
    caller.abort();
    assert!(caller.await.is_err_and(|error| error.is_cancelled()));

    let service = fixture.service.clone();
    let follow_up = tokio::spawn(async move { service.delete(existing).await });
    gate.release();
    follow_up
        .await
        .expect("follow-up task")
        .expect("follow-up delete");
    fixture.settle().await;

    let memory = fixture.memory_ids();
    assert_eq!(memory.len(), 1);
    assert_ne!(memory[0], existing);
    assert_eq!(fixture.disk_ids().await, memory);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancelled_while_waiting_for_lock_still_commits_consistently() {
    let (gate, hook) = WriteGate::install();
    let fixture = Fixture::new(&["AAAA", "AQEB"], hook).await;
    let (first, queued) = (fixture.ids[0], fixture.ids[1]);

    let service = fixture.service.clone();
    let holder = tokio::spawn(async move { service.delete(first).await });
    gate.wait_reached().await;

    let service = fixture.service.clone();
    let waiter = tokio::time::timeout(Duration::ZERO, service.delete(queued)).await;
    assert!(
        waiter.is_err(),
        "queued delete is still waiting for the lock"
    );
    // Let the detached commit enqueue on the lock before settle() does.
    tokio::time::sleep(Duration::from_millis(50)).await;

    gate.release();
    holder.await.expect("holder task").expect("holder delete");
    fixture.settle().await;

    assert_eq!(fixture.memory_ids(), Vec::<Uuid>::new());
    assert_eq!(fixture.disk_ids().await, Vec::<Uuid>::new());
}
