use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn manual_live_refresh_updates_registry_and_agent_view() -> Result<()> {
    let server = TestServer::start().await?;
    let node = server
        .issue_node("itest-refresh-01", "Integration Refresh 01")
        .await?;
    let mut agent = TestAgent::connect(&server, &node).await?;

    let (expires_at, refresh) = tokio::try_join!(
        server.request_live_token_refresh(&node.node_id),
        agent.wait_for_refresh_response(LIVE_REFRESH_TIMEOUT),
    )?;

    assert_eq!(refresh.expires_at, expires_at.to_rfc3339());
    assert_ne!(refresh.new_token, node.token);
    assert!(
        server
            .is_token_current(&node.node_id, &refresh.new_token)
            .await
    );
    assert!(
        server.is_token_current(&node.node_id, &node.token).await,
        "old token should remain usable during bounded refresh recovery grace"
    );

    agent.disconnect().await?;
    server.shutdown().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_live_refresh_keeps_each_node_consistent() -> Result<()> {
    let server = TestServer::start().await?;
    let node_a = server
        .issue_node("itest-refresh-02", "Integration Refresh 02")
        .await?;
    let node_b = server
        .issue_node("itest-refresh-03", "Integration Refresh 03")
        .await?;
    let node_c = server
        .issue_node("itest-refresh-04", "Integration Refresh 04")
        .await?;

    let mut agent_a = TestAgent::connect(&server, &node_a).await?;
    let mut agent_b = TestAgent::connect(&server, &node_b).await?;
    let mut agent_c = TestAgent::connect(&server, &node_c).await?;

    let (expires_a, expires_b, expires_c, refresh_a, refresh_b, refresh_c) = tokio::try_join!(
        server.request_live_token_refresh(&node_a.node_id),
        server.request_live_token_refresh(&node_b.node_id),
        server.request_live_token_refresh(&node_c.node_id),
        agent_a.wait_for_refresh_response(LIVE_REFRESH_TIMEOUT),
        agent_b.wait_for_refresh_response(LIVE_REFRESH_TIMEOUT),
        agent_c.wait_for_refresh_response(LIVE_REFRESH_TIMEOUT),
    )?;

    assert_eq!(refresh_a.expires_at, expires_a.to_rfc3339());
    assert_eq!(refresh_b.expires_at, expires_b.to_rfc3339());
    assert_eq!(refresh_c.expires_at, expires_c.to_rfc3339());

    for (node_id, old_token, new_token) in [
        (
            &node_a.node_id,
            node_a.token.as_str(),
            refresh_a.new_token.as_str(),
        ),
        (
            &node_b.node_id,
            node_b.token.as_str(),
            refresh_b.new_token.as_str(),
        ),
        (
            &node_c.node_id,
            node_c.token.as_str(),
            refresh_c.new_token.as_str(),
        ),
    ] {
        assert_ne!(new_token, old_token);
        assert!(server.is_token_current(node_id, new_token).await);
        assert!(
            server.is_token_current(node_id, old_token).await,
            "old token should remain usable during bounded refresh recovery grace"
        );
    }

    agent_a.disconnect().await?;
    agent_b.disconnect().await?;
    agent_c.disconnect().await?;
    server.shutdown().await
}
