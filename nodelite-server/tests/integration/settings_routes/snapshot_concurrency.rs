//! Snapshot commit order and registry reconciliation prevent deleted display nodes from returning.

use super::*;
use crate::snapshot::{
    SnapshotPersistCheckpoint, persist_current_snapshot, persist_snapshot_if_changed,
};
use crate::state::SharedState;
use tokio::time::timeout;

async fn add_node(harness: &SettingsHarness, node_id: &str) -> Result<()> {
    issue_node(
        &harness.state.shared.config().node_registry_path,
        IssueNodeRequest {
            node_id: node_id.to_string(),
            node_label: Some(node_id.to_string()),
            tags: vec![],
        },
    )
    .await?;
    harness.state.registry.reload().await?;
    harness
        .state
        .shared
        .register_node(
            synthetic_identity(node_id, node_id, "1", None, "edge"),
            None,
            None,
            None,
        )
        .await;
    Ok(())
}

#[tokio::test]
async fn periodic_snapshot_and_delete_commit_latest_view_in_order() -> Result<()> {
    let harness = SettingsHarness::new(readonly_auth(false, None)).await?;
    add_node(&harness, "snapshot-delete").await?;
    let shared = &harness.state.shared;
    let path = &shared.config().snapshot_path;
    persist_current_snapshot(shared, path).await?;
    let guard = shared.snapshot_write_lock.lock().await;
    let mut checkpoint = SnapshotPersistCheckpoint::default();
    let mut periodic = Box::pin(persist_snapshot_if_changed(shared, path, &mut checkpoint));
    assert!(
        timeout(Duration::from_millis(20), &mut periodic)
            .await
            .is_err()
    );
    let mut deletion = Box::pin(harness.app.clone().oneshot(json_delete_request(
        "/api/settings/agents/snapshot-delete",
        &basic_auth_header("secret"),
        None,
        json!({"current_password": "secret"}),
    )));
    assert!(
        timeout(Duration::from_millis(100), &mut deletion)
            .await
            .is_err()
    );
    assert!(shared.list_statuses().await.is_empty());
    drop(guard);
    assert!(periodic.await?);
    assert_eq!(deletion.await?.status(), StatusCode::OK);
    assert!(load_snapshot(path).await?.is_empty());
    assert!(!persist_snapshot_if_changed(shared, path, &mut checkpoint).await?);
    let restored = SharedState::new(Arc::new(shared.config().clone()));
    crate::startup::restore_snapshot_if_available(&restored, &harness.state.registry, path).await;
    assert!(restored.list_statuses().await.is_empty());
    assert!(
        harness
            .state
            .registry
            .list_registered_nodes()
            .await
            .is_empty()
    );
    harness.cleanup().await;
    Ok(())
}

#[tokio::test]
async fn restart_filters_stale_snapshot_against_authorized_registry() -> Result<()> {
    let harness = SettingsHarness::new(readonly_auth(false, None)).await?;
    add_node(&harness, "deleted").await?;
    add_node(&harness, "retained").await?;
    let shared = &harness.state.shared;
    let path = &shared.config().snapshot_path;
    persist_current_snapshot(shared, path).await?;
    harness.state.registry.remove_node("deleted").await?;
    // Simulate a crash after credential revocation but before the deletion snapshot.
    assert_eq!(load_snapshot(path).await?.len(), 2);
    let restored = SharedState::new(Arc::new(shared.config().clone()));
    crate::startup::restore_snapshot_if_available(&restored, &harness.state.registry, path).await;
    let statuses = restored.list_statuses().await;
    assert_eq!(statuses.len(), 1);
    assert_eq!(statuses[0].identity.node_id, "retained");
    harness.cleanup().await;
    Ok(())
}
