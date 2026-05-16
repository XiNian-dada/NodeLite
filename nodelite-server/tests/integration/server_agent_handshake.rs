use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn authenticates_agent_and_exposes_node_over_http() -> Result<()> {
    let server = TestServer::start().await?;
    let node = server.issue_node("itest-handshake-01", "Integration Handshake 01").await?;

    let mut agent = TestAgent::connect(&server, &node).await?;
    agent.send_fake_metrics(1).await?;

    let status = server
        .wait_for_node_uptime(&node.node_id, 1, TEST_TIMEOUT)
        .await?;
    assert!(status.online);
    assert_eq!(status.identity.node_label, node.node_label);

    let overview = server.overview().await?;
    assert_eq!(overview.total_nodes, 1);
    assert_eq!(overview.online_nodes, 1);

    let node_status = server.node_status(&node.node_id).await?;
    assert_eq!(node_status.identity.node_id, node.node_id);
    assert!(node_status.online);

    let nodes = server.nodes().await?;
    assert_eq!(nodes.len(), 1);
    assert_eq!(nodes[0].identity.node_id, node.node_id);

    agent.disconnect().await?;
    server.shutdown().await
}
