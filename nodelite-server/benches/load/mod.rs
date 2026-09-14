//! Load workloads compiled only by the custom cargo bench target.

mod diagnostics;
mod fake_agent;
mod history_matrix;
mod large_scale;
mod log_memory;
mod probes;
mod reconnect;
mod scenarios;
mod server;

use std::time::Duration;

use tokio::net::TcpStream;

const LOAD_TEST_TIMEOUT_SECS: u64 = 30;
const LOAD_TEST_METRICS_PER_NODE: u64 = 12;
const LOAD_TEST_OVERVIEW_PROBES: usize = 24;
const LOAD_TEST_READ_PROBES: usize = 20;
const LOAD_TEST_HISTORY_POINTS: usize = 360;
const LOAD_TEST_STEADY_METRICS_PER_NODE: u64 = 18;
const LOAD_TEST_STEADY_METRIC_DELAY_MS: u64 = 15;
const LOAD_TEST_STORM_CYCLES: usize = 4;
const LOAD_TEST_STORM_METRICS_PER_CYCLE: u64 = 6;
const LOAD_TEST_STORM_METRIC_DELAY_MS: u64 = 10;
const LOAD_TEST_STORM_READ_PROBES: usize = 12;
const LOAD_TEST_BASIC_AUTH: &str = "Basic dmlld2VyOnNlY3JldA==";

#[derive(Debug, Clone)]
struct AgentCredential {
    node_id: String,
    node_label: String,
    token: String,
}

#[derive(Debug)]
struct ScenarioResult {
    nodes: usize,
    metrics_total: usize,
    connect_ms: f64,
    settle_ms: f64,
    metrics_per_sec: f64,
    overview_p50_ms: f64,
    overview_p95_ms: f64,
    overview_max_ms: f64,
}

#[derive(Debug)]
struct ApiScenarioResult {
    nodes: usize,
    steady_metrics_total: usize,
    connect_ms: f64,
    settle_ms: f64,
    steady_metrics_per_sec: f64,
    history_seed_points: usize,
    overview: LatencySummary,
    nodes_api: LatencySummary,
    node_api: LatencySummary,
    history_api: LatencySummary,
}

#[derive(Debug)]
struct StormScenarioResult {
    nodes: usize,
    cycles: usize,
    sessions_total: usize,
    connect: LatencySummary,
    recover: LatencySummary,
    disconnect: LatencySummary,
    overview: LatencySummary,
    nodes_api: LatencySummary,
}

#[derive(Debug, Clone, Copy)]
struct LatencySummary {
    p50_ms: f64,
    p95_ms: f64,
    max_ms: f64,
}

#[derive(Debug, Clone, Copy)]
struct AgentWorkload {
    uptime_start: u64,
    metrics_per_node: u64,
    inter_message_delay: Duration,
    hold_after_send: Duration,
    disk_entries: usize,
}

type TestSocket = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<TcpStream>>;

pub async fn run(scenario: &str) -> anyhow::Result<()> {
    match scenario {
        "log-memory" => log_memory::run(false).await,
        "log-memory-sparse" => log_memory::run(true).await,
        "scaling" => scenarios::run_scaling_load_test().await,
        "api-surface" => scenarios::run_api_surface_load_test().await,
        "reconnect" => scenarios::run_reconnect_storm_load_test().await,
        "token-budget" => scenarios::run_token_verify_storm_load_test().await,
        "large-fleet" => large_scale::run_large_fleet_load_test().await,
        "dashboard" => large_scale::run_dashboard_fanout_load_test().await,
        "history-pressure" => large_scale::run_history_pressure_load_test().await,
        "history-matrix" => history_matrix::run_history_query_matrix().await,
        "history-matrix-child" if history_matrix::child_case_requested() => {
            history_matrix::run_history_query_matrix_child().await
        }
        "payload" => large_scale::run_payload_size_load_test().await,
        "read-write" => large_scale::run_concurrent_read_write_load_test().await,
        _ => anyhow::bail!("unknown benchmark scenario: {scenario}"),
    }
}
