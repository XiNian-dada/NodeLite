//! Authorization snapshots must follow durable commit order, not task completion order.

use std::path::PathBuf;
use std::sync::Barrier;
use std::sync::atomic::{AtomicBool, Ordering};
use std::task::Poll;

use super::*;
use crate::registry::storage::{
    RegistryGuard, load_registry_file_sync, migrate_registry_file, mutate_registry_file,
};

struct Fixture {
    path: PathBuf,
    registry: NodeRegistry,
    issued: crate::registry::IssueNodeResult,
}

impl Fixture {
    async fn new() -> Self {
        let suffix = crate::registry::token::generate_token().expect("random fixture suffix");
        let path = std::env::temp_dir()
            .join(format!("nodelite-registry-consistency-{suffix}"))
            .join("server.json");
        let issued = issue_node(&path, request())
            .await
            .expect("issue fixture node");
        let registry = NodeRegistry::load(&path).await.expect("load fixture");
        Self {
            path,
            registry,
            issued,
        }
    }

    fn snapshot(&self) -> RegistryFile {
        load_registry_file_sync(&self.path).expect("read fixture snapshot")
    }

    async fn assert_disk_matches_runtime(&self) {
        let disk =
            crate::registry::storage::load_registry_state_from_file(&self.path, self.snapshot())
                .expect("decode fixture snapshot");
        assert!(
            *self.registry.state.read().await == disk,
            "runtime must match the durable registry"
        );
    }

    fn write_legacy_snapshot(&self, version: u64) {
        let mut file = self.snapshot();
        file.version = version;
        file.nodes[0].token_hash.clear();
        file.nodes[0].token = self.issued.node_session_token.clone();
        file.nodes[0].token_generation = 0;
        let payload = serde_json::to_vec(&file).expect("serialize legacy fixture");
        std::fs::write(&self.path, payload).expect("write legacy fixture");
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
        let _ = std::fs::remove_file(self.path.with_file_name("server.json.lock"));
        if let Some(parent) = self.path.parent() {
            let _ = std::fs::remove_dir(parent);
        }
    }
}

fn request() -> IssueNodeRequest {
    IssueNodeRequest {
        node_id: "fixture-node".to_string(),
        node_label: None,
        tags: Vec::new(),
    }
}

#[tokio::test]
async fn stale_management_snapshot_cannot_restore_removed_node() {
    let fixture = Fixture::new().await;
    let stale = fixture.snapshot();
    let registry = fixture.registry.clone();
    let (release, waiting) = tokio::sync::oneshot::channel();
    let delayed = tokio::spawn(async move {
        waiting.await.expect("release delayed completion");
        registry.replace_state_from_file(stale).await
    });

    fixture
        .registry
        .remove_node("fixture-node")
        .await
        .expect("remove node");
    let revision = fixture.registry.registry_revision();
    release.send(()).expect("release stale snapshot");
    delayed
        .await
        .expect("delayed task")
        .expect("apply completion");

    assert_eq!(fixture.registry.count().await, 0, "node must stay removed");
    assert_eq!(fixture.registry.registry_revision(), revision);
    assert!(
        fixture
            .registry
            .authorize(
                &identity_for("fixture-node"),
                &fixture.issued.node_session_token
            )
            .await
            .is_err()
    );
    assert!(
        fixture
            .registry
            .consume_install_token(&fixture.issued.install_token)
            .await
            .expect("check consumed install")
            .is_none()
    );
    fixture.assert_disk_matches_runtime().await;
}

#[tokio::test]
async fn queued_snapshot_cannot_restore_revoked_credentials() {
    let fixture = Fixture::new().await;
    let identity = identity_for("fixture-node");
    let old_session = fixture
        .registry
        .authorize(&identity, &fixture.issued.node_session_token)
        .await
        .expect("prime authentication cache");
    let stale = fixture.snapshot();
    let rotated = issue_node(&fixture.path, request())
        .await
        .expect("rotate credentials");
    let latest = fixture.snapshot();
    let revision = fixture.registry.registry_revision();

    // Poll both writers while the lock is held so their queue order is deterministic.
    let guard = fixture.registry.state.write().await;
    let mut newer = Box::pin(fixture.registry.replace_state_from_file(latest));
    assert!(matches!(futures::poll!(newer.as_mut()), Poll::Pending));
    let mut older = Box::pin(fixture.registry.replace_state_from_file(stale));
    assert!(matches!(futures::poll!(older.as_mut()), Poll::Pending));
    drop(guard);
    newer.await.expect("apply newer commit");
    older.await.expect("finish delayed older commit");

    assert!(
        fixture
            .registry
            .authorize(&identity, &fixture.issued.node_session_token)
            .await
            .is_err(),
        "revoked token must stay revoked"
    );
    assert!(
        fixture
            .registry
            .session_token_status(
                "fixture-node",
                &fixture.issued.node_session_token,
                old_session.generation
            )
            .await
            .is_err()
    );
    fixture
        .registry
        .authorize(&identity, &rotated.node_session_token)
        .await
        .expect("current token must authorize");
    assert_eq!(fixture.registry.registry_revision(), revision + 1);
    fixture.assert_disk_matches_runtime().await;
}

#[tokio::test]
async fn stale_reload_preserves_revocation_and_observes_the_next_commit() {
    let fixture = Fixture::new().await;
    let (stale, fingerprint) =
        crate::registry::storage::load_registry_state_with_fingerprint(&fixture.path)
            .await
            .expect("prepare reload snapshot");
    let ((), removed) = mutate_registry_file(&fixture.path, RegistryGuard::AllowCreate, |file| {
        file.nodes.clear();
        file.install_sessions.clear();
        Ok(((), true))
    })
    .await
    .expect("commit removal");

    let guard = fixture.registry.state.write().await;
    let mut newer = Box::pin(fixture.registry.replace_state_from_file(removed));
    assert!(matches!(futures::poll!(newer.as_mut()), Poll::Pending));
    let mut older = Box::pin(
        fixture
            .registry
            .apply_loaded_state(stale, Some(fingerprint)),
    );
    assert!(matches!(futures::poll!(older.as_mut()), Poll::Pending));
    drop(guard);
    newer.await.expect("apply removal");
    assert!(!older.await, "stale reload must not be applied");

    assert_eq!(fixture.registry.count().await, 0);
    assert!(
        fixture
            .registry
            .reload_checkpoint
            .read()
            .await
            .fingerprint
            .is_none()
    );
    fixture.assert_disk_matches_runtime().await;

    let recreated = issue_node(&fixture.path, request())
        .await
        .expect("commit new node");
    assert!(
        fixture
            .registry
            .reload_if_file_changed()
            .await
            .expect("load newer commit")
    );
    let identity = identity_for("fixture-node");
    assert!(
        fixture
            .registry
            .authorize(&identity, &fixture.issued.node_session_token)
            .await
            .is_err()
    );
    fixture
        .registry
        .authorize(&identity, &recreated.node_session_token)
        .await
        .expect("new credential must authorize");
    fixture.assert_disk_matches_runtime().await;

    reset_registry_file_read_count(&fixture.path);
    assert!(
        !fixture
            .registry
            .reload_if_file_changed()
            .await
            .expect("unchanged file")
    );
    assert_eq!(registry_file_read_count(&fixture.path), 0);
}

#[tokio::test]
async fn duplicate_commit_does_not_invalidate_authorization_again() {
    let fixture = Fixture::new().await;
    let file = fixture.snapshot();
    let revision = fixture.registry.registry_revision();
    fixture
        .registry
        .replace_state_from_file(file)
        .await
        .expect("apply duplicate");
    assert!(!fixture.registry.reload().await.expect("reload duplicate"));
    assert_eq!(fixture.registry.registry_revision(), revision);
    fixture.assert_disk_matches_runtime().await;
}

#[tokio::test]
async fn registry_version_exhaustion_does_not_publish_a_mutation() {
    let fixture = Fixture::new().await;
    let mut file = fixture.snapshot();
    file.version = u64::MAX;
    let payload = serde_json::to_vec(&file).expect("serialize fixture");
    std::fs::write(&fixture.path, &payload).expect("write boundary fixture");

    let result = mutate_registry_file(&fixture.path, RegistryGuard::AllowCreate, |file| {
        file.nodes.clear();
        file.install_sessions.clear();
        Ok(((), true))
    })
    .await;

    assert!(result.is_err(), "commit versions must never stop advancing");
    assert!(
        std::fs::read(&fixture.path).expect("read boundary fixture") == payload,
        "failed mutation must leave disk unchanged"
    );
}

#[tokio::test]
async fn delayed_legacy_migration_preserves_later_removal() {
    let fixture = Fixture::new().await;
    fixture.write_legacy_snapshot(fixture.snapshot().version);
    let legacy = fixture.snapshot();
    assert!(!legacy.nodes[0].token.is_empty());

    fixture
        .registry
        .remove_node("fixture-node")
        .await
        .expect("remove legacy node");
    let migrated = migrate_registry_file(&fixture.path, RegistryGuard::AllowCreate)
        .await
        .expect("finish delayed migration");
    assert!(migrated.version > legacy.version);
    fixture
        .registry
        .replace_state_from_file(migrated)
        .await
        .expect("apply migration");
    assert_eq!(fixture.registry.count().await, 0);
    fixture.assert_disk_matches_runtime().await;
}

#[tokio::test]
async fn migration_retry_cannot_restore_a_concurrently_removed_node() {
    let fixture = Fixture::new().await;
    fixture.write_legacy_snapshot(fixture.snapshot().version);
    let prepared = Arc::new(tokio::sync::Notify::new());
    let resume = Arc::new(Barrier::new(2));
    let ready = Arc::clone(&prepared);
    let release = Arc::clone(&resume);
    let first_attempt = Arc::new(AtomicBool::new(true));
    let path = fixture.path.clone();
    let migration = tokio::spawn(async move {
        mutate_registry_file(&path, RegistryGuard::AllowCreate, move |file| {
            if first_attempt.swap(false, Ordering::SeqCst) {
                ready.notify_one();
                release.wait();
            }
            crate::registry::token::migrate_legacy_tokens(file)?;
            Ok(((), true))
        })
        .await
    });

    tokio::time::timeout(Duration::from_secs(10), prepared.notified())
        .await
        .expect("migration must reach the preparation barrier");
    let removed = fixture.registry.remove_node("fixture-node").await;
    resume.wait();
    removed.expect("commit removal while migration is preparing");
    let ((), migrated) = migration
        .await
        .expect("migration task")
        .expect("retry migration");
    fixture
        .registry
        .replace_state_from_file(migrated)
        .await
        .expect("apply retried migration");
    assert_eq!(fixture.registry.count().await, 0);
    fixture.assert_disk_matches_runtime().await;
}

#[tokio::test]
async fn delayed_legacy_migration_preserves_later_credentials() {
    let fixture = Fixture::new().await;
    fixture.write_legacy_snapshot(fixture.snapshot().version);
    let legacy = fixture.snapshot();
    assert!(!legacy.nodes[0].token.is_empty());

    let rotated = issue_node(&fixture.path, request())
        .await
        .expect("rotate legacy credential");
    let migrated = migrate_registry_file(&fixture.path, RegistryGuard::AllowCreate)
        .await
        .expect("finish delayed migration");
    assert!(migrated.version > legacy.version);
    fixture
        .registry
        .replace_state_from_file(migrated)
        .await
        .expect("apply migration");
    let identity = identity_for("fixture-node");
    assert!(
        fixture
            .registry
            .authorize(&identity, &fixture.issued.node_session_token)
            .await
            .is_err()
    );
    fixture
        .registry
        .authorize(&identity, &rotated.node_session_token)
        .await
        .expect("new credential survives migration");
    fixture.assert_disk_matches_runtime().await;
}

#[tokio::test]
async fn legacy_migration_version_exhaustion_does_not_replace_disk() {
    let fixture = Fixture::new().await;
    fixture.write_legacy_snapshot(u64::MAX);
    let before = std::fs::read(&fixture.path).expect("read legacy fixture");

    assert!(NodeRegistry::load(&fixture.path).await.is_err());
    assert!(std::fs::read(&fixture.path).expect("read preserved fixture") == before);
}
