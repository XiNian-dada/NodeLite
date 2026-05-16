use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn manual_live_refresh_updates_registry_and_agent_view() -> Result<()> {
    let server = TestServer::start().await?;
    let node = server.issue_node("itest-refresh-01", "Integration Refresh 01").await?;
    let mut agent = TestAgent::connect(&server, &node).await?;

    let expires_at = server.request_live_token_refresh(&node.node_id).await?;
    let refresh = agent.wait_for_refresh_response(TEST_TIMEOUT).await?;

    assert_eq!(refresh.expires_at, expires_at.to_rfc3339());
    assert_ne!(refresh.new_token, node.token);
    assert!(server.is_token_current(&node.node_id, &refresh.new_token).await);
    assert!(!server.is_token_current(&node.node_id, &node.token).await);

    agent.disconnect().await?;
    server.shutdown().await
}
