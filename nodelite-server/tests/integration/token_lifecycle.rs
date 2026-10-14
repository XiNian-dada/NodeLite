use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn manual_refresh_waits_for_durable_confirmation_and_failed_disk_write_keeps_node_online()
-> Result<()> {
    let server = TestServer::start().await?;
    let node = server.issue_node("itest-durable-01", "Durable 01").await?;
    let mut agent = TestAgent::connect(&server, &node).await?;
    let mut request = Box::pin(server.request_live_token_refresh(&node.node_id));
    let candidate = tokio::select! {
        candidate = agent.receive_refresh_response(LIVE_REFRESH_TIMEOUT) => candidate?,
        reply = &mut request => panic!("request completed before Agent confirmation: {reply:?}"),
    };
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(20), &mut request)
            .await
            .is_err()
    );
    agent.acknowledge_refresh(&candidate, false).await?;
    assert!(request.await.is_err());
    assert!(server.is_token_current(&node.node_id, &node.token).await);
    agent.send_fake_metrics(2).await?;
    assert!(
        server
            .wait_for_node_uptime(&node.node_id, 2, TEST_TIMEOUT)
            .await?
            .online
    );
    let (expires_at, retried) = tokio::try_join!(
        server.request_live_token_refresh(&node.node_id),
        agent.wait_for_refresh_response(LIVE_REFRESH_TIMEOUT),
    )?;
    assert_eq!(
        retried.refresh_id, candidate.refresh_id,
        "retries must resend the same saved candidate"
    );
    assert_eq!(retried.expires_at, expires_at.to_rfc3339());
    agent.disconnect().await?;
    server.shutdown().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn saved_candidate_reconnects_when_connection_drops_before_acknowledgement() -> Result<()> {
    let server = TestServer::start().await?;
    let node = server
        .issue_node("itest-lost-ack-01", "Lost ACK 01")
        .await?;
    let mut agent = TestAgent::connect(&server, &node).await?;
    let (result, candidate) =
        tokio::join!(server.request_live_token_refresh(&node.node_id), async {
            let candidate = agent.receive_refresh_response(LIVE_REFRESH_TIMEOUT).await?;
            agent.disconnect().await?;
            Ok::<_, anyhow::Error>(candidate)
        },);
    assert!(result.is_err());
    let candidate = candidate?;
    let saved = crate::test_support::TestNode {
        token: candidate.new_token,
        ..node
    };
    let mut restarted = TestAgent::connect(&server, &saved).await?;
    restarted.send_fake_metrics(3).await?;
    assert!(
        server
            .wait_for_node_uptime(&saved.node_id, 3, TEST_TIMEOUT)
            .await?
            .online
    );
    restarted.disconnect().await?;
    server.shutdown().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn legacy_agent_is_not_rotated_without_durable_confirmation_support() -> Result<()> {
    let server = TestServer::start().await?;
    let node = server.issue_node("itest-legacy-01", "Legacy 01").await?;
    let mut agent = TestAgent::connect_with_confirmation(&server, &node, false).await?;
    let error = server
        .request_live_token_refresh(&node.node_id)
        .await
        .expect_err("legacy renewal must fail safely");
    assert!(error.to_string().contains("Agent upgrade required"));
    assert!(server.is_token_current(&node.node_id, &node.token).await);
    agent.send_fake_metrics(4).await?;
    assert!(
        server
            .wait_for_node_uptime(&node.node_id, 4, TEST_TIMEOUT)
            .await?
            .online
    );
    agent.disconnect().await?;
    server.shutdown().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn duplicate_old_acknowledgement_does_not_cancel_the_next_transaction() -> Result<()> {
    let server = TestServer::start().await?;
    let node = server
        .issue_node("itest-duplicate-ack-01", "Duplicate ACK")
        .await?;
    let mut agent = TestAgent::connect(&server, &node).await?;
    let (_, first) = tokio::try_join!(
        server.request_live_token_refresh(&node.node_id),
        agent.wait_for_refresh_response(LIVE_REFRESH_TIMEOUT)
    )?;
    let (result, _) = tokio::try_join!(server.request_live_token_refresh(&node.node_id), async {
        let second = agent.receive_refresh_response(LIVE_REFRESH_TIMEOUT).await?;
        agent.acknowledge_refresh(&first, true).await?;
        agent.acknowledge_refresh(&second, true).await?;
        Ok::<_, anyhow::Error>(second)
    })?;
    assert!(result > chrono::Utc::now());
    agent.send_fake_metrics(5).await?;
    assert!(
        server
            .wait_for_node_uptime(&node.node_id, 5, TEST_TIMEOUT)
            .await?
            .online
    );
    agent.disconnect().await?;
    server.shutdown().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn exhausted_candidate_budget_keeps_valid_current_credentials_and_metrics_online()
-> Result<()> {
    let server = TestServer::start().await?;
    let node = server
        .issue_node("itest-candidate-limit-01", "Candidate Limit")
        .await?;
    let path = server.registry.path();
    let mut file: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path)?)?;
    file["nodes"][0]["token_expires_at"] =
        serde_json::json!((chrono::Utc::now() + chrono::Duration::days(6)).to_rfc3339());
    std::fs::write(path, serde_json::to_vec(&file)?)?;
    server.registry.reload().await?;
    for _ in 0..crate::registry::renewal::MAX_PENDING_TOKENS {
        server
            .registry
            .prepare_token_refresh(&node.node_id, 1)
            .await?;
    }
    let mut agent = TestAgent::connect(&server, &node).await?;
    for uptime in 1..=3 {
        agent.send_fake_metrics(uptime).await?;
        assert!(
            server
                .wait_for_node_uptime(&node.node_id, uptime, TEST_TIMEOUT)
                .await?
                .online
        );
    }
    assert!(
        server
            .request_live_token_refresh(&node.node_id)
            .await
            .expect_err("limit must require operator action")
            .to_string()
            .contains("unconfirmed credential limit")
    );
    agent.send_fake_metrics(6).await?;
    assert!(
        server
            .wait_for_node_uptime(&node.node_id, 6, TEST_TIMEOUT)
            .await?
            .online
    );
    agent.disconnect().await?;
    server.shutdown().await
}

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
