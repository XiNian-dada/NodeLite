use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn multiple_nodes_can_connect_and_report_together() -> Result<()> {
    let server = TestServer::start().await?;
    let mut issued_nodes = Vec::new();
    for index in 0..4 {
        issued_nodes.push(
            server
                .issue_node(
                    &format!("itest-concurrent-{index:02}"),
                    &format!("Integration Concurrent {index:02}"),
                )
                .await?,
        );
    }

    let mut agents = try_join_all(issued_nodes.iter().map(|node| TestAgent::connect(&server, node)))
        .await?;
    for (index, agent) in agents.iter_mut().enumerate() {
        agent.send_fake_metrics(index as u64 + 1).await?;
    }
    for (index, node) in issued_nodes.iter().enumerate() {
        server
            .wait_for_node_uptime(&node.node_id, index as u64 + 1, TEST_TIMEOUT)
            .await?;
    }

    let overview = server.overview().await?;
    assert_eq!(overview.total_nodes, issued_nodes.len());
    assert_eq!(overview.online_nodes, issued_nodes.len());

    let statuses = server.nodes().await?;
    assert_eq!(statuses.len(), issued_nodes.len());
    assert!(statuses.iter().all(|status| status.online));

    for agent in agents {
        agent.disconnect().await?;
    }
    server.shutdown().await
}
