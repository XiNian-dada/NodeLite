use std::net::SocketAddr;
use std::time::Duration;

use nodelite_proto::uses_insecure_remote_url;
use tokio::task::JoinHandle;
use tokio::time::{MissedTickBehavior, interval};
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};
use url::Url;

use crate::agent_logs::AgentLogStore;
use crate::app_state::ServerReadiness;
use crate::history::HistoryStore;
use crate::registry::NodeRegistry;
use crate::state::SharedState;

/// 后台任务:每秒扫描一次注册表,把超时节点标记为离线。
pub(crate) fn spawn_stale_reaper(
    shared: SharedState,
    shutdown: CancellationToken,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut ticker = interval(Duration::from_secs(1));
        // 进程或主机被挂起后,interval 默认会"补打"积压 tick;这里改为延后下一次,
        // 避免恢复瞬间连续多次扫描全表(对大规模注册表是无谓的 CPU 抖动)。
        ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                _ = shutdown.cancelled() => break,
                _ = ticker.tick() => {
                    let count = shared.mark_stale().await;
                    if count > 0 {
                        info!(count, "marked stale nodes offline");
                    }
                }
            }
        }
    })
}

/// 后台任务:每秒检查一次注册表文件是否有外部更改(例如 CLI 颁发了新节点)。
pub(crate) fn spawn_registry_reloader(
    registry: NodeRegistry,
    history: HistoryStore,
    agent_logs: AgentLogStore,
    readiness: ServerReadiness,
    shutdown: CancellationToken,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut ticker = interval(Duration::from_secs(1));
        // 挂起恢复后只想做一次最近态的 reload,而不是连续 N 次磁盘 IO。
        ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                _ = shutdown.cancelled() => break,
                _ = ticker.tick() => {
                    match registry.reload_if_file_changed().await {
                        Ok(true) => {
                            readiness.mark_registry_reload_healthy(true);
                            let enrolled_nodes = registry.count().await;
                            let node_ids = registry.node_ids().await;
                            let cleaned_history_nodes = history.forget_missing(&node_ids).await;
                            let cleaned_agent_log_nodes = agent_logs.forget_missing(&node_ids).await;
                            info!(
                                registry_path = %registry.path().display(),
                                enrolled_nodes,
                                cleaned_history_nodes,
                                cleaned_agent_log_nodes,
                                "reloaded node registry",
                            );
                        }
                        Ok(false) => {
                            readiness.mark_registry_reload_healthy(true);
                        }
                        Err(error) => {
                            readiness.mark_registry_reload_healthy(false);
                            warn!(
                                error = ?error,
                                registry_path = %registry.path().display(),
                                "failed to reload node registry; keeping previous in-memory snapshot",
                            );
                        }
                    }
                }
            }
        }
    })
}

/// 在监听非回环地址但仍然使用 `http://` 公网基址时,周期性输出 TLS 警告。
pub(crate) fn spawn_insecure_transport_warning(
    public_base_url: String,
    listen: SocketAddr,
    insecure_transport_warn_interval_secs: u64,
    shutdown: CancellationToken,
) -> Option<JoinHandle<()>> {
    if !uses_insecure_remote_public_base_url(&public_base_url, listen) {
        return None;
    }

    Some(tokio::spawn(async move {
        let mut ticker = interval(Duration::from_secs(insecure_transport_warn_interval_secs));
        // 警告是节流型日志,跳过错过的 tick 即可,不要在恢复后连续 burst 多条相同警告。
        ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                _ = shutdown.cancelled() => break,
                _ = ticker.tick() => {
                    warn!(
                        listen = %listen,
                        public_base_url = %public_base_url,
                        "server is configured without TLS; use an https:// public_base_url and terminate TLS in front of NodeLite",
                    );
                }
            }
        }
    }))
}

pub(crate) fn uses_insecure_remote_public_base_url(
    public_base_url: &str,
    listen: SocketAddr,
) -> bool {
    let Ok(url) = Url::parse(public_base_url) else {
        return false;
    };
    if url.scheme() != "http" {
        return false;
    }
    if !listen.ip().is_loopback() {
        return true;
    }

    uses_insecure_remote_url(public_base_url, "http")
}

#[cfg(test)]
mod tests {
    use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::time::{SystemTime, UNIX_EPOCH};

    use tokio::time::{Duration, timeout};
    use tokio_util::sync::CancellationToken;

    use super::{
        spawn_insecure_transport_warning, spawn_registry_reloader,
        uses_insecure_remote_public_base_url,
    };
    use crate::AppState;
    use crate::test_support::test_server_config;

    fn temp_dir(label: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be monotonic enough")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("nodelite-background-{label}-{unique}"));
        std::fs::create_dir_all(&dir).expect("temp dir should exist");
        dir
    }

    async fn background_state_fixture(label: &str) -> (AppState, PathBuf) {
        let temp_dir = temp_dir(label);
        let registry_path = temp_dir.join("server.json");
        let config = test_server_config(
            SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 8080)),
            "https://monitor.example.com".to_string(),
            registry_path,
            temp_dir.join("history.sqlite3"),
            temp_dir.join("snapshot.json"),
        );
        let state = AppState::test_fixture(config.into(), Arc::new(temp_dir.join("server.toml")))
            .await
            .expect("state fixture should build");
        (state, temp_dir)
    }

    #[test]
    fn insecure_public_base_url_handles_invalid_urls_and_listener_scope() {
        assert!(!uses_insecure_remote_public_base_url(
            "not a url",
            SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 8080)),
        ));
        assert!(uses_insecure_remote_public_base_url(
            "http://monitor.example.com",
            SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 8080)),
        ));
        assert!(!uses_insecure_remote_public_base_url(
            "http://127.0.0.1:8080",
            SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 8080)),
        ));
    }

    #[tokio::test]
    async fn insecure_transport_warning_only_spawns_for_remote_plaintext_urls() {
        let shutdown = CancellationToken::new();
        assert!(
            spawn_insecure_transport_warning(
                "https://monitor.example.com".to_string(),
                SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 8080)),
                1,
                shutdown.clone(),
            )
            .is_none()
        );
        assert!(
            spawn_insecure_transport_warning(
                "http://127.0.0.1:8080".to_string(),
                SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 8080)),
                1,
                shutdown.clone(),
            )
            .is_none()
        );

        let handle = spawn_insecure_transport_warning(
            "http://monitor.example.com".to_string(),
            SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 8080)),
            1,
            shutdown.clone(),
        )
        .expect("remote plaintext urls should spawn warning task");

        shutdown.cancel();
        timeout(Duration::from_secs(1), handle)
            .await
            .expect("warning task should stop promptly")
            .expect("warning task should shut down cleanly");
    }

    #[tokio::test]
    async fn registry_reloader_marks_readiness_healthy_when_registry_is_unchanged() {
        let (state, temp_dir) = background_state_fixture("registry-unchanged").await;
        state.readiness.mark_registry_reload_healthy(false);
        let shutdown = CancellationToken::new();
        let handle = spawn_registry_reloader(
            state.registry.clone(),
            state.history.clone(),
            state.agent_logs.clone(),
            state.readiness.clone(),
            shutdown.clone(),
        );

        timeout(Duration::from_secs(1), async {
            loop {
                if state.readiness.registry_reload_healthy() {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("registry reload should restore readiness");

        shutdown.cancel();
        timeout(Duration::from_secs(1), handle)
            .await
            .expect("registry reloader should stop promptly")
            .expect("registry reloader should stop cleanly");
        let _ = tokio::fs::remove_dir_all(temp_dir).await;
    }

    #[tokio::test]
    async fn registry_reloader_marks_readiness_unhealthy_after_reload_error() {
        let (state, temp_dir) = background_state_fixture("registry-error").await;
        tokio::fs::create_dir_all(state.registry.path())
            .await
            .expect("registry path should be replaceable with a directory");

        let shutdown = CancellationToken::new();
        let handle = spawn_registry_reloader(
            state.registry.clone(),
            state.history.clone(),
            state.agent_logs.clone(),
            state.readiness.clone(),
            shutdown.clone(),
        );

        timeout(Duration::from_secs(1), async {
            loop {
                if !state.readiness.registry_reload_healthy() {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("registry reload failure should degrade readiness");

        shutdown.cancel();
        timeout(Duration::from_secs(1), handle)
            .await
            .expect("registry reloader should stop promptly")
            .expect("registry reloader should stop cleanly");
        let _ = tokio::fs::remove_dir_all(temp_dir).await;
    }
}
