//! P3 告警预览与升级冲突的 HTTP 回归测试。

use super::route_surface_tests::{readyz_test_state, readyz_test_state_with};
use crate::{
    handlers::alert_settings,
    history::TrafficSample,
    registry::{IssueNodeRequest, NodeServiceMetadata, TrafficAccounting, issue_node},
    test_support::{fake_snapshot, synthetic_identity},
};
use axum::{
    Router,
    body::{Body, to_bytes},
    extract::State,
    http::{Request, StatusCode, header},
    response::IntoResponse,
};
use chrono::{Duration, Utc};
use serde_json::Value;
use std::sync::Arc;
use tower::util::ServiceExt;

#[tokio::test]
async fn preview_matches_runtime_window_and_traffic_evaluation() {
    let (state, dir) = readyz_test_state("preview-parity").await;
    issue_node(
        &state.shared.config().node_registry_path,
        IssueNodeRequest {
            node_id: "hk-01".into(),
            node_label: Some("Hong Kong".into()),
            tags: Vec::new(),
        },
    )
    .await
    .expect("issue node");
    state.registry.reload().await.expect("reload registry");
    state
        .registry
        .update_service_metadata(
            "hk-01",
            NodeServiceMetadata {
                service_expires_at: None,
                service_unlimited: false,
                renewal_price: None,
                traffic_limit_bytes: Some(1000),
                traffic_accounting: TrafficAccounting::Outbound,
                traffic_throttle_kbps: None,
            },
        )
        .await
        .expect("traffic limit");
    let identity = synthetic_identity("hk-01", "Hong Kong", "0.1.0", None, "test");
    let session = state.shared.register_node(identity, None, None, None).await;
    let mut snapshot = fake_snapshot(100);
    snapshot.cpu_usage_percent = Some(99.0);
    state
        .shared
        .update_snapshot("hk-01", session, snapshot)
        .await
        .expect("snapshot");
    let now = Utc::now();
    let connection =
        rusqlite::Connection::open(&state.shared.config().history_db_path).expect("history");
    for i in 0..5 {
        connection.execute("INSERT INTO history_points(node_id,recorded_at,cpu_usage_percent,memory_used_percent) VALUES (?1,?2,?3,50)", rusqlite::params!["hk-01", (now - Duration::minutes(i)).timestamp(), 10.0]).expect("cold window");
    }
    drop(connection);
    for (tx, uptime) in [(0, 100), (950, 101)] {
        state
            .history
            .record_traffic(
                "hk-01",
                TrafficSample {
                    rx_bytes: 0,
                    tx_bytes: tx,
                    uptime_secs: uptime,
                },
                TrafficAccounting::Outbound,
                true,
            )
            .await
            .expect("usage");
    }
    let mut config = state.alerting.read().await.as_ref().clone();
    let mut cpu = nodelite_proto::AlertRuleConfig {
        id: "cpu".into(),
        name: "CPU".into(),
        enabled: true,
        metric: nodelite_proto::AlertMetric::CpuUsagePercent,
        comparator: nodelite_proto::AlertComparator::Gt,
        threshold: 90,
        window_minutes: 5,
        severity: nodelite_proto::AlertSeverity::Warning,
        scope_mode: nodelite_proto::AlertScopeMode::All,
        node_ids: Vec::new(),
        tags: Vec::new(),
        delivery: Vec::new(),
        cooldown_minutes: 30,
        send_resolved: true,
    };
    cpu.id = "cpu".into();
    cpu.enabled = true;
    cpu.metric = nodelite_proto::AlertMetric::CpuUsagePercent;
    cpu.threshold = 90;
    cpu.window_minutes = 5;
    let mut traffic = cpu.clone();
    traffic.id = "traffic".into();
    traffic.metric = nodelite_proto::AlertMetric::TrafficUsagePercent;
    config.rules = vec![cpu, traffic];
    *state.alerting.write().await = Arc::new(config.clone());
    let runtime = crate::alerts::evaluate_alert_rules_with_history(
        &state.shared,
        &state.history,
        &state.registry,
        &config.rules,
        now,
    )
    .await;
    assert_eq!(runtime.len(), 1);
    assert_eq!(runtime[0].rule_id, "traffic");
    let response = alert_settings(State(state.clone())).await.into_response();
    let json: Value = serde_json::from_slice(
        &to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body"),
    )
    .expect("json");
    assert_eq!(
        json["preview"]["triggered_rules"]
            .as_array()
            .expect("rules")
            .len(),
        1
    );
    assert_eq!(json["preview"]["triggered_rules"][0]["rule_id"], "traffic");
    state.shutdown.cancel();
    state.history.shutdown().await;
    state.audit_log.shutdown().await;
    drop(state);
    std::fs::remove_dir_all(dir).expect("remove fixtures");
}

#[tokio::test]
async fn second_http_update_returns_conflict_and_preserves_active_log() {
    use std::os::fd::AsRawFd;
    let (mut state, dir) = readyz_test_state_with("update-conflict", |_, config| {
        config.readonly_auth = Some(nodelite_proto::ReadonlyAuthConfig {
            username: "viewer".into(),
            password: "fixture-password".into(),
            enable_2fa: false,
            totp_secret: None,
        });
    })
    .await;
    state.config_path = Arc::new(dir.join("server.toml"));
    let log = dir.join("server-web-update.log");
    std::fs::write(&log, b"first upgrade log").expect("log");
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("server-web-update.log.lock"))
        .expect("lock file");
    assert_eq!(unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX) }, 0);
    let app: Router = crate::startup::build_router(state.clone());
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/settings/update/server")
                .header(
                    header::AUTHORIZATION,
                    "Basic dmlld2VyOmZpeHR1cmUtcGFzc3dvcmQ=",
                )
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    r#"{"mode":"stable","current_password":"fixture-password"}"#,
                ))
                .expect("request"),
        )
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert_eq!(std::fs::read(log).expect("log"), b"first upgrade log");
    drop(lock);
    state.shutdown.cancel();
    state.history.shutdown().await;
    state.audit_log.shutdown().await;
    drop(state);
    std::fs::remove_dir_all(dir).expect("remove fixtures");
}
