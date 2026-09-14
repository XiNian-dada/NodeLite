use std::io::ErrorKind;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

use super::AgentCredential;
use nodelite_server::bench_support::BenchmarkRuntime;
use nodelite_server::bench_support::HistoryStore;
use nodelite_server::bench_support::SharedState;
use nodelite_server::bench_support::{IssueNodeRequest, issue_node};
use nodelite_server::bench_support::{test_server_config, test_ws_config};

pub(super) struct TestServer {
    pub(super) addr: SocketAddr,
    pub(super) shared: SharedState,
    pub(super) history: HistoryStore,
    shutdown_tx: Option<tokio::sync::oneshot::Sender<()>>,
    server_handle: JoinHandle<Result<(), std::io::Error>>,
    runtime: BenchmarkRuntime,
    temp_dir: PathBuf,
    history_path: PathBuf,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct HistoryArtifactBytes {
    pub(super) db: u64,
    pub(super) wal: u64,
    pub(super) shm: u64,
}

impl TestServer {
    pub(super) async fn start(node_count: usize) -> Result<(Self, Vec<AgentCredential>)> {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should move forward")
            .as_nanos();
        let temp_dir = std::env::temp_dir().join(format!("nodelite-load-test-{unique}"));
        tokio::fs::create_dir_all(&temp_dir)
            .await
            .with_context(|| format!("create temp dir {}", temp_dir.display()))?;

        let listener =
            TcpListener::bind(SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))).await?;
        let addr = listener.local_addr()?;
        let registry_path = temp_dir.join("server.json");
        let history_path = temp_dir.join("history.sqlite3");
        let snapshot_path = temp_dir.join("snapshot.json");

        let mut credentials = Vec::with_capacity(node_count);
        for index in 0..node_count {
            let node_id = format!("load-node-{index:03}");
            let node_label = format!("Load Node {index:03}");
            let issued = issue_node(
                &registry_path,
                IssueNodeRequest {
                    node_id: node_id.clone(),
                    node_label: Some(node_label.clone()),
                    tags: vec!["load-test".to_string()],
                },
            )
            .await
            .with_context(|| format!("issue node {node_id}"))?;
            credentials.push(AgentCredential {
                node_id,
                node_label,
                token: issued.node_session_token,
            });
        }

        let mut config = test_server_config(
            addr,
            format!("http://{addr}"),
            registry_path,
            history_path.clone(),
            snapshot_path,
        );
        config.ws = test_ws_config(node_count.saturating_add(32), node_count.saturating_add(32));
        config.stale_after_secs = 20;
        let config = std::sync::Arc::new(config);
        let runtime =
            BenchmarkRuntime::new(config, std::sync::Arc::new(temp_dir.join("server.toml")))
                .await?;
        let history = runtime.history();
        let shared = runtime.shared();
        let app = runtime.router();

        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
        let server_handle = tokio::spawn(async move {
            axum::serve(
                listener,
                app.into_make_service_with_connect_info::<SocketAddr>(),
            )
            .with_graceful_shutdown(async {
                let _ = shutdown_rx.await;
            })
            .await
        });

        Ok((
            Self {
                addr,
                shared,
                history,
                shutdown_tx: Some(shutdown_tx),
                server_handle,
                runtime,
                temp_dir,
                history_path,
            },
            credentials,
        ))
    }

    pub(super) fn registry(&self) -> nodelite_server::bench_support::NodeRegistry {
        self.runtime.registry()
    }

    pub(super) async fn history_artifact_bytes(&self) -> Result<HistoryArtifactBytes> {
        Ok(HistoryArtifactBytes {
            db: file_len_or_zero(&self.history_path).await?,
            wal: file_len_or_zero(&PathBuf::from(format!(
                "{}-wal",
                self.history_path.display()
            )))
            .await?,
            shm: file_len_or_zero(&PathBuf::from(format!(
                "{}-shm",
                self.history_path.display()
            )))
            .await?,
        })
    }

    pub(super) async fn shutdown(mut self) -> Result<()> {
        if let Some(shutdown_tx) = self.shutdown_tx.take() {
            let _ = shutdown_tx.send(());
        }
        let result = self
            .server_handle
            .await
            .map_err(|error| anyhow!("join server task: {error}"))?;
        result.map_err(|error| anyhow!("server task: {error}"))?;
        self.runtime.shutdown().await;
        let _ = tokio::fs::remove_dir_all(&self.temp_dir).await;
        Ok(())
    }
}

async fn file_len_or_zero(path: &PathBuf) -> Result<u64> {
    match tokio::fs::metadata(path).await {
        Ok(metadata) => Ok(metadata.len()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(0),
        Err(error) => Err(error).with_context(|| format!("stat {}", path.display())),
    }
}
