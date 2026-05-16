use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn metrics_flow_reaches_status_and_history_endpoints() -> Result<()> {
    let server = TestServer::start().await?;
    let node = server.issue_node("itest-metrics-01", "Integration Metrics 01").await?;
    let mut agent = TestAgent::connect(&server, &node).await?;

    agent.send_fake_metrics(7).await?;

    let status = server
        .wait_for_node_uptime(&node.node_id, 7, TEST_TIMEOUT)
        .await?;
    assert_eq!(
        status.snapshot.as_ref().map(|snapshot| snapshot.uptime_secs),
        Some(7),
    );

    let history = server
        .wait_for_history_points(&node.node_id, 1, TEST_TIMEOUT)
        .await?;
    assert!(!history.is_empty());
    assert!(history.iter().all(|point| point.node_id == node.node_id));
    assert!(history[0].cpu_usage_percent >= 0.0);

    agent.disconnect().await?;
    server.shutdown().await
}
