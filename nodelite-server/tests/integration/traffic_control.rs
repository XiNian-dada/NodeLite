//! Capability and execution reports travel through the authenticated protocol into settings.

use super::*;
use nodelite_proto::{TrafficControlState, TrafficControlStatus, TrafficControlUnavailableReason};

#[tokio::test]
async fn traffic_control_report_is_visible_in_settings_and_expires_with_the_connection()
-> Result<()> {
    let server = TestServer::start().await?;
    let node = server
        .issue_node("traffic-report", "Traffic report")
        .await?;
    let mut agent = TestAgent::connect(&server, &node).await?;
    let report = TrafficControlStatus {
        state: TrafficControlState::Unavailable,
        reason: Some(TrafficControlUnavailableReason::MissingTc),
        desired_rate_kbps: Some(1000),
        applied_rate_kbps: None,
    };
    agent.send_traffic_control_status(report.clone()).await?;
    tokio::time::timeout(TEST_TIMEOUT, async {
        loop {
            let settings: serde_json::Value = server.fetch_json("/api/settings").await?;
            if settings["agents"][0]["traffic_control"] == serde_json::to_value(&report)? {
                break Ok::<_, anyhow::Error>(());
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await??;
    agent.disconnect().await?;
    server
        .wait_for_node_offline(&node.node_id, TEST_TIMEOUT)
        .await?;
    let settings: serde_json::Value = server.fetch_json("/api/settings").await?;
    assert!(settings["agents"][0]["traffic_control"].is_null());
    server.shutdown().await
}
