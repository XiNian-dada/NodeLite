//! 根据套餐账本决定 Agent 应处于的限速状态。

use crate::AppState;
use crate::history::TrafficSample;
use crate::registry::TrafficAccounting;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ThrottleDecision {
    /// 账本不可信时无从判断是否超额。`rate_kbps: None` 会让 Agent 解除限速，
    /// 所以这里不下发任何指令，让 Agent 保持上一次生效的状态。
    Hold,
    Apply(Option<u64>),
}

pub(super) async fn record_and_decide(
    state: &AppState,
    node_id: &str,
    sample: TrafficSample,
) -> ThrottleDecision {
    let Some(quota) = state.registry.traffic_quota(node_id).await else {
        let _ = state
            .history
            .record_traffic(node_id, sample, TrafficAccounting::Bidirectional, false)
            .await;
        return ThrottleDecision::Apply(None);
    };
    match state
        .history
        .record_traffic(node_id, sample, quota.accounting, true)
        .await
    {
        Ok(usage) => ThrottleDecision::Apply(usage.and_then(|usage| {
            usage_reached_throttle_threshold(usage.used_bytes, quota.limit_bytes)
                .then_some(quota.throttle_kbps)
                .flatten()
        })),
        Err(_) => decide_without_ledger(
            quota.throttle_kbps,
            state.history.traffic_throttle_hold_expired(),
        ),
    }
}

/// 保持只针对“可能已超额”。已取消限速配置或已跨月时不再需要账本就能判断，必须解除，
/// 否则账本长期故障会让限速永远无法结束。
fn decide_without_ledger(throttle_kbps: Option<u64>, hold_expired: bool) -> ThrottleDecision {
    if throttle_kbps.is_none() || hold_expired {
        ThrottleDecision::Apply(None)
    } else {
        ThrottleDecision::Hold
    }
}

fn usage_reached_throttle_threshold(used_bytes: u64, limit_bytes: u64) -> bool {
    u128::from(used_bytes) * 100 >= u128::from(limit_bytes) * 95
}

#[cfg(test)]
mod tests {
    use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::{
        ThrottleDecision, decide_without_ledger, record_and_decide,
        usage_reached_throttle_threshold,
    };
    use crate::AppState;
    use crate::history::TrafficSample;
    use crate::registry::{IssueNodeRequest, NodeServiceMetadataPatch, issue_node};
    use crate::test_support::test_server_config;

    const NODE: &str = "quota-node";

    fn sample(rx_bytes: u64, uptime_secs: u64) -> TrafficSample {
        TrafficSample {
            rx_bytes,
            tx_bytes: 0,
            uptime_secs,
        }
    }

    async fn state_with_quota() -> (AppState, PathBuf) {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be available")
            .as_nanos();
        let temp_dir = std::env::temp_dir().join(format!("nodelite-traffic-quota-{unique}"));
        std::fs::create_dir_all(&temp_dir).expect("temp dir should exist");
        let config = test_server_config(
            SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 8080)),
            "http://127.0.0.1:8080".to_string(),
            temp_dir.join("registry.json"),
            temp_dir.join("history.sqlite3"),
            temp_dir.join("snapshot.json"),
        );
        let state = AppState::test_fixture(
            Arc::new(config),
            Arc::new(PathBuf::from("config/server.toml")),
        )
        .await
        .expect("state fixture should build");
        issue_node(
            state.registry.path(),
            IssueNodeRequest {
                node_id: NODE.into(),
                node_label: None,
                tags: vec![],
            },
        )
        .await
        .expect("node should be issued");
        state
            .registry
            .reload()
            .await
            .expect("registry should reload");
        state
            .registry
            .update_service_metadata_patch(
                NODE,
                NodeServiceMetadataPatch {
                    traffic_limit_bytes: Some(Some(1_000)),
                    traffic_throttle_kbps: Some(Some(64)),
                    ..Default::default()
                },
            )
            .await
            .expect("quota should be stored");
        (state, temp_dir)
    }

    #[test]
    fn traffic_throttle_threshold_is_ninety_five_percent() {
        assert!(!usage_reached_throttle_threshold(949, 1_000));
        assert!(usage_reached_throttle_threshold(950, 1_000));
        assert!(usage_reached_throttle_threshold(u64::MAX, u64::MAX));
    }

    #[test]
    fn hold_ends_on_month_rollover_or_when_throttling_is_no_longer_configured() {
        assert_eq!(
            decide_without_ledger(Some(64), false),
            ThrottleDecision::Hold
        );
        assert_eq!(
            decide_without_ledger(Some(64), true),
            ThrottleDecision::Apply(None)
        );
        assert_eq!(
            decide_without_ledger(None, false),
            ThrottleDecision::Apply(None)
        );
    }

    #[tokio::test]
    async fn unavailable_ledger_holds_instead_of_clearing_the_throttle() {
        let (state, temp_dir) = state_with_quota().await;

        assert_eq!(
            record_and_decide(&state, NODE, sample(0, 10)).await,
            ThrottleDecision::Apply(None)
        );
        assert_eq!(
            record_and_decide(&state, NODE, sample(950, 20)).await,
            ThrottleDecision::Apply(Some(64))
        );

        state.history.shutdown().await;
        assert_eq!(
            record_and_decide(&state, NODE, sample(960, 30)).await,
            ThrottleDecision::Hold
        );
        // 未配置套餐的节点不依赖账本，限速必须能照常解除。
        assert_eq!(
            record_and_decide(&state, "no-quota", sample(960, 30)).await,
            ThrottleDecision::Apply(None)
        );

        state.audit_log.shutdown().await;
        drop(state);
        let _ = std::fs::remove_dir_all(temp_dir);
    }
}
