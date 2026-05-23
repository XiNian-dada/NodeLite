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
