use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn newer_session_supersedes_older_connection_and_keeps_latest_snapshot() -> Result<()> {
    let server = TestServer::start().await?;
    let node = server.issue_node("itest-reconnect-01", "Integration Reconnect 01").await?;

    let mut first_agent = TestAgent::connect(&server, &node).await?;
    first_agent.send_fake_metrics(1).await?;
    server
        .wait_for_node_uptime(&node.node_id, 1, TEST_TIMEOUT)
        .await?;

    let mut second_agent = TestAgent::connect(&server, &node).await?;
    second_agent.send_fake_metrics(99).await?;
    server
        .wait_for_node_uptime(&node.node_id, 99, TEST_TIMEOUT)
        .await?;

    first_agent.send_fake_metrics(2).await?;
    let status = server
        .wait_for_node_uptime(&node.node_id, 99, TEST_TIMEOUT)
        .await?;
    assert_eq!(
        status.snapshot.as_ref().map(|snapshot| snapshot.uptime_secs),
        Some(99),
    );

    second_agent.disconnect().await?;
    server.wait_for_node_offline(&node.node_id, TEST_TIMEOUT).await?;
    server.shutdown().await
}
