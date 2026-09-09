//! Node revocation removes the active billing ledger without erasing retained history.

use super::*;
use crate::registry::TrafficAccounting;

const NODE: &str = "quota-node";

async fn enroll(harness: &SettingsHarness, node_id: &str) -> Result<()> {
    issue_node(
        harness.state.registry.path(),
        IssueNodeRequest {
            node_id: node_id.into(),
            node_label: None,
            tags: vec![],
        },
    )
    .await?;
    harness.state.registry.reload().await?;
    Ok(())
}

async fn record_usage(state: &crate::AppState, node_id: &str) {
    state
        .history
        .record_traffic(node_id, 100, 200, TrafficAccounting::Bidirectional, true)
        .await;
    let usage = state
        .history
        .record_traffic(node_id, 400, 800, TrafficAccounting::Bidirectional, true)
        .await
        .expect("usage");
    assert_eq!(usage.used_bytes, 900);
}

fn ledger_rows(state: &crate::AppState) -> Result<i64> {
    let connection = rusqlite::Connection::open(&state.shared.config().history_db_path)?;
    Ok(connection.query_row("SELECT COUNT(*) FROM traffic_usage", [], |r| r.get(0))?)
}

async fn delete(harness: &SettingsHarness) -> Result<Response<Body>> {
    Ok(harness
        .app
        .clone()
        .oneshot(json_delete_request(
            "/api/settings/agents/quota-node",
            &basic_auth_header("secret"),
            None,
            json!({"current_password": "secret"}),
        ))
        .await?)
}

#[tokio::test]
async fn deleting_a_node_commits_ledger_removal_before_success_and_reenrollment_starts_at_zero()
-> Result<()> {
    let harness = SettingsHarness::new(readonly_auth(false, None)).await?;
    enroll(&harness, NODE).await?;
    record_usage(&harness.state, NODE).await;
    seed_retained_history(&harness.state)?;
    let response = delete(&harness).await?;
    assert_status(response.status(), StatusCode::OK, response).await?;
    assert!(harness.state.history.traffic_usages().await.is_empty());
    assert_eq!(
        ledger_rows(&harness.state)?,
        0,
        "pending upserts cannot follow a successful delete"
    );
    harness.state.history.shutdown().await;
    let restarted = crate::AppState::test_fixture(
        Arc::new(harness.state.shared.config().clone()),
        Arc::new(harness.config_path.clone()),
    )
    .await?;
    assert!(restarted.history.traffic_usages().await.is_empty());
    enroll(&harness, NODE).await?;
    restarted.registry.reload().await?;
    let usage = restarted
        .history
        .record_traffic(NODE, 10_000, 20_000, TrafficAccounting::Bidirectional, true)
        .await
        .expect("new baseline");
    assert_eq!(usage.used_bytes, 0);
    assert_retained_history(&restarted)?;
    restarted.shutdown.cancel();
    restarted.history.shutdown().await;
    restarted.audit_log.shutdown().await;
    harness.cleanup().await;
    Ok(())
}

#[tokio::test]
async fn startup_discards_orphaned_traffic_ledgers_and_keeps_registered_nodes() -> Result<()> {
    let harness = SettingsHarness::new(readonly_auth(false, None)).await?;
    for id in [NODE, "retained"] {
        enroll(&harness, id).await?;
        record_usage(&harness.state, id).await;
    }
    seed_retained_history(&harness.state)?;
    harness.state.history.shutdown().await;
    assert_eq!(ledger_rows(&harness.state)?, 2);
    harness.state.registry.remove_node(NODE).await?;
    let restarted = crate::AppState::test_fixture(
        Arc::new(harness.state.shared.config().clone()),
        Arc::new(harness.config_path.clone()),
    )
    .await?;
    let usages = restarted.history.traffic_usages().await;
    assert_eq!(usages.len(), 1);
    assert_eq!(usages[0].node_id, "retained");
    assert_eq!(usages[0].used_bytes, 900);
    assert_eq!(ledger_rows(&restarted)?, 1);
    assert_retained_history(&restarted)?;
    restarted.shutdown.cancel();
    restarted.history.shutdown().await;
    restarted.audit_log.shutdown().await;
    harness.cleanup().await;
    Ok(())
}

#[tokio::test]
async fn failed_ledger_cleanup_never_reports_successful_deletion() -> Result<()> {
    let harness = SettingsHarness::new(readonly_auth(false, None)).await?;
    enroll(&harness, NODE).await?;
    record_usage(&harness.state, NODE).await;
    tokio::time::timeout(Duration::from_secs(5), async {
        while ledger_rows(&harness.state).expect("ledger count") == 0 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await?;
    let connection = rusqlite::Connection::open(&harness.state.shared.config().history_db_path)?;
    connection.execute_batch("CREATE TRIGGER refuse_traffic_delete BEFORE DELETE ON traffic_usage BEGIN SELECT RAISE(ABORT, 'injected deletion failure'); END;")?;
    assert_eq!(
        delete(&harness).await?.status(),
        StatusCode::INTERNAL_SERVER_ERROR
    );
    assert_eq!(
        harness.state.registry.count().await,
        0,
        "credential revocation must survive cleanup failure"
    );
    assert!(harness.state.history.traffic_usages().await.is_empty());
    connection.execute_batch("DROP TRIGGER refuse_traffic_delete;")?;
    harness.state.history.forget_traffic(NODE).await?;
    assert_eq!(ledger_rows(&harness.state)?, 0);
    drop(connection);
    harness.cleanup().await;
    Ok(())
}

fn seed_retained_history(state: &crate::AppState) -> Result<()> {
    let connection = rusqlite::Connection::open(&state.shared.config().history_db_path)?;
    connection.execute(
        "INSERT INTO history_points (node_id, recorded_at, memory_used_percent) VALUES (?1, ?2, 50)",
        rusqlite::params![NODE, Utc::now().timestamp()],
    )?;
    Ok(())
}

fn assert_retained_history(state: &crate::AppState) -> Result<()> {
    let connection = rusqlite::Connection::open(&state.shared.config().history_db_path)?;
    let count: i64 = connection.query_row(
        "SELECT COUNT(*) FROM history_points WHERE node_id = ?1",
        [NODE],
        |r| r.get(0),
    )?;
    assert_eq!(
        count, 1,
        "history retention is independent of active billing state"
    );
    Ok(())
}
