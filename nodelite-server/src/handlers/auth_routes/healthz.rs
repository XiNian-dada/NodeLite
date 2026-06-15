//! 进程存活与服务就绪探针。

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Serialize;

use crate::AppState;

#[derive(Serialize)]
struct ReadyzResponse {
    status: &'static str,
    ready: bool,
    problems: Vec<&'static str>,
    checks: ReadyzChecks,
    signals: ReadyzSignals,
}

#[derive(Serialize)]
struct ReadyzChecks {
    history_available: bool,
    registry_reload_healthy: bool,
}

#[derive(Serialize)]
struct ReadyzSignals {
    audit_enabled: bool,
    audit_available: bool,
    history_dropped_writes: u64,
    audit_dropped_writes: u64,
    audit_write_failures: u64,
    history_queue_depth: u64,
    history_queue_capacity: u64,
    audit_queue_depth: u64,
    audit_queue_capacity: u64,
    ws_active_connections: usize,
    ws_connection_capacity: usize,
    ws_max_connections_per_ip: usize,
    browser_ws_active_connections: usize,
    browser_ws_connection_capacity: usize,
    browser_ws_max_connections_per_ip: usize,
}

/// 健康检查接口,始终返回 200。
pub(crate) async fn healthz() -> StatusCode {
    StatusCode::OK
}

/// 就绪检查接口:保留 200/503 语义,同时返回结构化诊断方便排障。
pub(crate) async fn readyz(State(state): State<AppState>) -> Response {
    let history_available = state.readiness.history_available();
    let registry_reload_healthy = state.readiness.registry_reload_healthy();
    let ready = history_available && registry_reload_healthy;
    let audit_enabled = state.audit_log.enabled();
    let audit_available = state.audit_log.is_available().await;
    let history_dropped_writes = state.history.dropped_writes();
    let audit_dropped_writes = state.audit_log.dropped_writes();
    let audit_write_failures = state.audit_log.write_failures();
    let (history_queue_depth, history_queue_capacity) = state.history.writer_queue_metrics().await;
    let (audit_queue_depth, audit_queue_capacity) = state.audit_log.writer_queue_metrics().await;
    let ws_snapshot = state.ws_admission.snapshot();
    let browser_ws_snapshot = state.browser_ws_admission.snapshot();

    let mut problems = Vec::new();
    if !history_available {
        problems.push("history_unavailable");
    }
    if !registry_reload_healthy {
        problems.push("registry_reload_unhealthy");
    }
    if audit_enabled && !audit_available {
        problems.push("audit_unavailable");
    }
    if history_dropped_writes > 0 {
        problems.push("history_dropped_writes");
    }
    if audit_dropped_writes > 0 {
        problems.push("audit_dropped_writes");
    }
    if audit_write_failures > 0 {
        problems.push("audit_write_failures");
    }

    let response = ReadyzResponse {
        status: if problems.is_empty() {
            "ok"
        } else {
            "degraded"
        },
        ready,
        problems,
        checks: ReadyzChecks {
            history_available,
            registry_reload_healthy,
        },
        signals: ReadyzSignals {
            audit_enabled,
            audit_available,
            history_dropped_writes,
            audit_dropped_writes,
            audit_write_failures,
            history_queue_depth,
            history_queue_capacity,
            audit_queue_depth,
            audit_queue_capacity,
            ws_active_connections: ws_snapshot.active_connections,
            ws_connection_capacity: ws_snapshot.max_total_connections,
            ws_max_connections_per_ip: ws_snapshot.max_connections_per_ip,
            browser_ws_active_connections: browser_ws_snapshot.active_connections,
            browser_ws_connection_capacity: browser_ws_snapshot.max_total_connections,
            browser_ws_max_connections_per_ip: browser_ws_snapshot.max_connections_per_ip,
        },
    };
    let status = if ready {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    (status, Json(response)).into_response()
}
