//! Exercise SQLite failures below queue capacity and verify public diagnostics recover.

use axum::body::to_bytes;
use axum::extract::State;
use axum::http::StatusCode;
use serde_json::Value;

use super::*;
use crate::AppState;
use crate::handlers::{metrics, readyz};
use crate::test_support::test_server_config;

#[tokio::test]
async fn readonly_history_writes_degrade_diagnostics_and_recover() {
    check_failure_and_recovery(false).await;
}

#[tokio::test]
async fn full_history_database_degrades_diagnostics_and_recovers() {
    check_failure_and_recovery(true).await;
}

async fn check_failure_and_recovery(full: bool) {
    let path = temp_history_db_path(if full { "full" } else { "readonly" });
    let state = test_state(&path).await;
    inject_fault(&state.history, full).await;
    // One large row needs new pages under SQLITE_FULL, without filling the writer queue.
    let node_id = if full {
        "x".repeat(100_000)
    } else {
        "failed-node".into()
    };
    state
        .history
        .record_status(&fake_status_for(&node_id, Utc::now()))
        .await;
    wait_for(&state.history, |m| m.failures == 1).await;
    assert_degraded(&state).await;
    assert_realtime_updates_continue(&state).await;

    {
        let mut guard = state.history.write_connection.lock().await;
        let connection = guard.as_mut().expect("write connection");
        connection
            .pragma_update(None, "query_only", false)
            .expect("writable");
        connection
            .pragma_update(None, "max_page_count", 1_000_000)
            .expect("restore capacity");
    }
    state
        .history
        .record_status(&fake_status_for("recovered", Utc::now()))
        .await;
    wait_for(&state.history, |m| m.last_success_at > 0).await;
    let recovered = diagnostics(&state).await;
    assert_eq!(recovered["status"], "ok");
    assert_eq!(recovered["signals"]["history_write_degraded"], false);
    assert_eq!(
        state.history.write_metrics().failures,
        1,
        "counters remain cumulative"
    );
    state.shutdown.cancel();
    state.history.shutdown().await;
    state.audit_log.shutdown().await;
    let connection = rusqlite::Connection::open(&path).expect("read persisted history");
    let count: i64 = connection
        .query_row("SELECT COUNT(*) FROM history_points", [], |r| r.get(0))
        .expect("count");
    assert_eq!(count, 1);
    drop(connection);
    drop(state);
    std::fs::remove_dir_all(path.parent().expect("temp directory")).expect("cleanup");
}

async fn inject_fault(store: &HistoryStore, full: bool) {
    let mut guard = store.write_connection.lock().await;
    let connection = guard.as_mut().expect("write connection");
    if full {
        let pages: u64 = connection
            .pragma_query_value(None, "page_count", |r| r.get(0))
            .expect("page count");
        connection
            .pragma_update(None, "max_page_count", pages)
            .expect("limit disk capacity");
    } else {
        connection
            .pragma_update(None, "query_only", true)
            .expect("make readonly");
    }
}

async fn wait_for(
    store: &HistoryStore,
    predicate: impl Fn(super::super::HistoryWriteMetrics) -> bool,
) {
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while !predicate(store.write_metrics()) {
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("writer should make progress");
}

async fn diagnostics(state: &AppState) -> Value {
    let response = readyz(State(state.clone())).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("diagnostics body");
    serde_json::from_slice(&body).expect("diagnostics JSON")
}

async fn test_state(path: &std::path::Path) -> AppState {
    let mut config = test_server_config(
        "127.0.0.1:0".parse().expect("address"),
        "http://127.0.0.1".into(),
        path.with_file_name("registry.json"),
        path.to_path_buf(),
        path.with_file_name("snapshot.json"),
    );
    config.history_writer_batch_max = 1;
    config.audit.enabled = false;
    AppState::test_fixture(
        std::sync::Arc::new(config),
        std::sync::Arc::new(path.with_file_name("server.toml")),
    )
    .await
    .expect("state")
}

async fn assert_degraded(state: &AppState) {
    let failed = state.history.write_metrics();
    assert_eq!(failed.lost_samples, 1);
    assert_eq!(failed.last_success_at, 0);
    assert!(failed.degraded);
    assert_eq!(state.history.dropped_writes(), 0);
    assert_eq!(state.history.writer_queue_metrics().await.0, 0);
    assert!(
        state.history.is_available(),
        "real-time admission must remain available"
    );
    let payload = diagnostics(state).await;
    assert_eq!(payload["status"], "degraded");
    assert_eq!(payload["ready"], true);
    assert_eq!(payload["signals"]["history_write_failures"], 1);
    assert_eq!(payload["signals"]["history_lost_samples"], 1);
    assert!(
        payload["problems"]
            .as_array()
            .expect("problems")
            .contains(&Value::from("history_write_failed"))
    );
    let response = metrics(State(state.clone())).await;
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("metrics body");
    let body = std::str::from_utf8(&body).expect("metrics text");
    assert!(body.contains("nodelite_history_write_failures_total 1"));
    assert!(body.contains("nodelite_history_lost_samples_total 1"));
}

async fn assert_realtime_updates_continue(state: &AppState) {
    let status = fake_status_for("live-node", Utc::now());
    let session = state
        .shared
        .register_node(status.identity, None, None, None)
        .await;
    let mut snapshot = status.snapshot.expect("snapshot");
    snapshot.cpu_usage_percent = Some(63.0);
    let updated = state
        .shared
        .update_snapshot("live-node", session, snapshot)
        .await
        .expect("live update survives history failure");
    assert_eq!(
        updated.snapshot.expect("latest snapshot").cpu_usage_percent,
        Some(63.0)
    );
}
