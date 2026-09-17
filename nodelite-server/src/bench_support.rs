//! Opt-in adapters for external benchmarks without exposing runtime internals in release builds.

use std::path::PathBuf;
use std::sync::Arc;

use axum::Router;
use axum::middleware::from_fn_with_state;
use axum::routing::get;
use nodelite_proto::ServerConfig;

use crate::AppState;
use crate::handlers::{
    metrics, node_history, node_logs, node_status, nodes, overview, require_readonly_auth,
};

pub use crate::agent_logs::{AgentLogStats, AgentLogStore};
pub use crate::history::HistoryStore;
pub use crate::registry::{IssueNodeRequest, NodeRegistry, issue_node};
pub use crate::state::SharedState;
pub use crate::test_fixtures::{
    fake_snapshot_at, send_wire_message, synthetic_identity, test_server_config, test_ws_config,
    wait_for_authenticated_notice,
};

#[derive(Debug, thiserror::Error)]
#[error("initialize benchmark runtime: {0}")]
pub struct BenchmarkError(#[from] anyhow::Error);

pub struct BenchmarkRuntime(AppState);

impl BenchmarkRuntime {
    pub async fn new(
        config: Arc<ServerConfig>,
        path: Arc<PathBuf>,
    ) -> Result<Self, BenchmarkError> {
        Ok(Self(AppState::test_fixture(config, path).await?))
    }

    pub fn history(&self) -> HistoryStore {
        self.0.history.clone()
    }
    pub fn shared(&self) -> SharedState {
        self.0.shared.clone()
    }
    pub fn registry(&self) -> NodeRegistry {
        self.0.registry.clone()
    }

    pub fn router(&self) -> Router {
        let protected = Router::new()
            .route("/api/overview", get(overview))
            .route("/metrics", get(metrics))
            .route("/api/nodes", get(nodes))
            .route("/api/nodes/{node_id}", get(node_status))
            .route("/api/nodes/{node_id}/history", get(node_history))
            .route("/api/nodes/{node_id}/logs", get(node_logs))
            .route_layer(from_fn_with_state(self.0.clone(), require_readonly_auth));
        Router::new()
            .route("/ws", get(crate::ws::ws_handler))
            .merge(protected)
            .with_state(self.0.clone())
    }

    pub async fn shutdown(self) {
        self.0.shutdown.cancel();
        self.0.history.shutdown().await;
        self.0.audit_log.shutdown().await;
    }
}

pub fn process_resident_memory_bytes() -> Option<u64> {
    crate::handlers::process_resident_memory_bytes()
}

pub fn token_verify_metrics(registry: &NodeRegistry) -> crate::registry::TokenVerifyMetrics {
    registry.token_verify_metrics()
}

pub fn api_cache_metrics(shared: &SharedState) -> crate::handlers::metrics_routes::ApiCacheMetrics {
    shared.api_cache_metrics()
}

pub async fn writer_queue_metrics(history: &HistoryStore) -> (u64, u64) {
    history.writer_queue_metrics().await
}

pub fn history_with_default_read_cache(
    path: PathBuf,
    timeout_secs: u64,
    concurrency: usize,
) -> HistoryStore {
    HistoryStore::new_with_default_read_cache(path, timeout_secs, concurrency)
}
