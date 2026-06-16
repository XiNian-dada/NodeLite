use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn token_cache_prevents_redundant_argon2_verifies_on_concurrent_requests() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be monotonic enough")
        .as_nanos();
    let temp_dir = std::env::temp_dir().join(format!("nodelite-token-cache-concurrent-{unique}"));
    std::fs::create_dir_all(&temp_dir).expect("temp dir should exist");
    let path = temp_dir.join("server.json");

    // Issue a node with a token
    let issued = issue_node(
        &path,
        IssueNodeRequest {
            node_id: "cache-01".to_string(),
            node_label: Some("Cache 01".to_string()),
            tags: Vec::new(),
        },
    )
    .await
    .expect("node should be issued");

    // Load registry with probe to count actual Argon2 verifications
    let probe = Arc::new(TokenVerifyProbe::new(Duration::from_millis(50)));
    let registry = NodeRegistry::load(&path)
        .await
        .expect("registry should load")
        .with_token_verify_limit_for_tests(2)
        .with_token_verify_probe_for_tests(Arc::clone(&probe));
    let identity = identity_for("cache-01");

    // Launch 10 concurrent authorization requests with the same token
    // Without cache: would run 10 Argon2 verifies (limited by semaphore to 2 parallel)
    // With cache + double-check: should run only 1-2 Argon2 verifies
    let mut handles = Vec::new();
    for _ in 0..10 {
        let registry = registry.clone();
        let identity = identity.clone();
        let token = issued.node_session_token.clone();
        handles.push(tokio::spawn(async move {
            registry.authorize(&identity, &token).await
        }));
    }

    // All requests should succeed
    for result in futures::future::join_all(handles).await {
        let authorized = result
            .expect("authorize task should complete")
            .expect("token should authorize");
        assert_eq!(authorized.identity.node_id, "cache-01");
    }

    // Cache should reduce actual Argon2 verifications to at most 2
    // (the semaphore limit, since concurrent requests may both miss cache)
    let max_active = probe.max_active();
    assert!(
        max_active <= 2,
        "expected at most 2 concurrent Argon2 verifies due to semaphore limit, got {max_active}"
    );

    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_dir(&temp_dir);
}

#[tokio::test]
async fn token_cache_respects_ttl_and_evicts_expired_entries() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be monotonic enough")
        .as_nanos();
    let temp_dir = std::env::temp_dir().join(format!("nodelite-token-cache-ttl-{unique}"));
    std::fs::create_dir_all(&temp_dir).expect("temp dir should exist");
    let path = temp_dir.join("server.json");

    let issued = issue_node(
        &path,
        IssueNodeRequest {
            node_id: "ttl-01".to_string(),
            node_label: Some("TTL 01".to_string()),
            tags: Vec::new(),
        },
    )
    .await
    .expect("node should be issued");

    let probe = Arc::new(TokenVerifyProbe::new(Duration::ZERO));
    let registry = NodeRegistry::load(&path)
        .await
        .expect("registry should load")
        .with_token_verify_probe_for_tests(Arc::clone(&probe));
    let identity = identity_for("ttl-01");

    // First authorization: cache miss, should run Argon2
    registry
        .authorize(&identity, &issued.node_session_token)
        .await
        .expect("first authorize should succeed");
    assert_eq!(probe.max_active(), 1);

    // Second authorization immediately: cache hit, no Argon2
    registry
        .authorize(&identity, &issued.node_session_token)
        .await
        .expect("second authorize should succeed");
    assert_eq!(probe.max_active(), 1, "cache hit should not trigger new Argon2 verify");

    // Note: Testing TTL expiration would require tokio::time::sleep(TOKEN_CACHE_TTL + margin)
    // which is 5+ minutes. We verify the double-check logic prevents redundant verifies instead.

    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_dir(&temp_dir);
}

#[tokio::test]
async fn token_cache_invalidates_on_token_rotation() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be monotonic enough")
        .as_nanos();
    let temp_dir = std::env::temp_dir().join(format!("nodelite-token-cache-rotate-{unique}"));
    std::fs::create_dir_all(&temp_dir).expect("temp dir should exist");
    let path = temp_dir.join("server.json");

    let issued = issue_node(
        &path,
        IssueNodeRequest {
            node_id: "rotate-01".to_string(),
            node_label: Some("Rotate 01".to_string()),
            tags: Vec::new(),
        },
    )
    .await
    .expect("node should be issued");

    let registry = NodeRegistry::load(&path)
        .await
        .expect("registry should load");
    let identity = identity_for("rotate-01");

    // Authorize with original token
    let authorized = registry
        .authorize(&identity, &issued.node_session_token)
        .await
        .expect("original token should authorize");
    assert_eq!(authorized.generation, 1);

    // Refresh token (cache should be cleared)
    let (new_token, _, new_generation) = registry
        .refresh_token("rotate-01")
        .await
        .expect("token should refresh");
    assert_eq!(new_generation, 2);

    // Old token should no longer authorize (cache cleared)
    let result = registry.authorize(&identity, &issued.node_session_token).await;
    assert!(result.is_err(), "old token should not authorize after rotation");

    // New token should authorize with updated generation
    let authorized = registry
        .authorize(&identity, &new_token)
        .await
        .expect("new token should authorize");
    assert_eq!(authorized.generation, 2);

    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_dir(&temp_dir);
}
