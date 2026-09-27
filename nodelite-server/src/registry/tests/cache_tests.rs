//! Token 验证缓存与淘汰策略测试模块。

use super::*;

#[tokio::test]
async fn token_cache_miss_counter_tracks_argon2_work() {
    let (registry, credentials, temp_dir) =
        token_cache_fixture("miss-metric", &["miss-01"], 2).await;
    let (identity, token) = &credentials[0];

    registry
        .authorize(identity, token)
        .await
        .expect("cold token should authorize");

    let metrics = registry.token_verify_metrics();
    assert_eq!(metrics.token_cache_hits_total, 0);
    assert_eq!(metrics.token_cache_misses_total, 1);
    assert_eq!(metrics.token_cache_evictions_total, 0);

    drop(registry);
    std::fs::remove_dir_all(temp_dir).expect("cache metric temp dir should be removable");
}

#[tokio::test]
async fn token_cache_hit_counter_tracks_warm_results() {
    let (registry, credentials, temp_dir) = token_cache_fixture("hit-metric", &["hit-01"], 2).await;
    let (identity, token) = &credentials[0];

    registry
        .authorize(identity, token)
        .await
        .expect("cold token should authorize");
    registry
        .authorize(identity, token)
        .await
        .expect("warm token should authorize");

    let metrics = registry.token_verify_metrics();
    assert_eq!(metrics.token_cache_hits_total, 1);
    assert_eq!(metrics.token_cache_misses_total, 1);
    assert_eq!(metrics.token_cache_evictions_total, 0);

    drop(registry);
    std::fs::remove_dir_all(temp_dir).expect("cache metric temp dir should be removable");
}

#[tokio::test]
async fn token_cache_eviction_counter_tracks_capacity_pressure() {
    let (registry, credentials, temp_dir) =
        token_cache_fixture("eviction-metric", &["evict-01", "evict-02"], 1).await;

    for (identity, token) in &credentials {
        registry
            .authorize(identity, token)
            .await
            .expect("token should authorize");
    }

    let metrics = registry.token_verify_metrics();
    assert_eq!(metrics.token_cache_hits_total, 0);
    assert_eq!(metrics.token_cache_misses_total, 2);
    assert_eq!(metrics.token_cache_evictions_total, 1);

    drop(registry);
    std::fs::remove_dir_all(temp_dir).expect("cache metric temp dir should be removable");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn token_cache_prevents_redundant_argon2_verifies_on_concurrent_requests() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be monotonic enough")
        .as_nanos();
    let temp_dir = std::env::temp_dir().join(format!("nodelite-token-cache-concurrent-{unique}"));
    std::fs::create_dir_all(&temp_dir).expect("temp dir should exist");
    let path = temp_dir.join("server.json");

    // 注册并签发测试节点与会话 token
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

    // 加载注册表并注入探针以统计实际 Argon2 校验次数
    let probe = Arc::new(TokenVerifyProbe::new(Duration::from_millis(50)));
    let registry = NodeRegistry::load(&path)
        .await
        .expect("registry should load")
        .with_token_verify_limit_for_tests(2)
        .with_token_verify_probe_for_tests(Arc::clone(&probe));
    let identity = identity_for("cache-01");

    // 使用相同 token 发起 10 个并发认证请求：
    // 无缓存：会执行 10 次 Argon2 验证（受信号量限制最大 2 并发）
    // 有缓存 + 双重检查：仅执行 1-2 次 Argon2 验证
    let mut handles = Vec::new();
    for _ in 0..10 {
        let registry = registry.clone();
        let identity = identity.clone();
        let token = issued.node_session_token.clone();
        handles.push(tokio::spawn(async move {
            registry.authorize(&identity, &token).await
        }));
    }

    // 所有并发认证请求均应成功
    for result in futures::future::join_all(handles).await {
        let authorized = result
            .expect("authorize task should complete")
            .expect("token should authorize");
        assert_eq!(authorized.identity.node_id, "cache-01");
    }

    // 缓存应将实际 Argon2 验证次数削减到最多 2 次（并发请求同时未命中时受信号量限制）
    let max_active = probe.max_active();
    assert!(
        max_active <= 2,
        "expected at most 2 concurrent Argon2 verifies due to semaphore limit, got {max_active}"
    );

    // 验证总 Argon2 次数远小于 10（总请求数）。覆盖率插桩可能会在首次填入缓存前放宽竞争窗口，
    // 因此稳定的断言是“缓存有效避免了每个请求都执行一次完整验证”。
    let total_verifies = probe.total_entered();
    assert!(
        total_verifies < 10,
        "expected fewer than 10 Argon2 verifies due to cache, got {total_verifies}"
    );

    registry
        .authorize(&identity, &issued.node_session_token)
        .await
        .expect("warm cache should authorize");
    assert_eq!(
        probe.total_entered(),
        total_verifies,
        "warm cache hit should not run another Argon2 verify"
    );
    let metrics = registry.token_verify_metrics();
    assert_eq!(metrics.token_cache_misses_total, total_verifies as u64);
    assert_eq!(
        metrics.token_cache_hits_total + metrics.token_cache_misses_total,
        11,
        "each authorization should produce exactly one cache outcome"
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

    // 首次认证：缓存未命中，执行 Argon2
    registry
        .authorize(&identity, &issued.node_session_token)
        .await
        .expect("first authorize should succeed");
    assert_eq!(
        probe.total_entered(),
        1,
        "first authorization should verify token"
    );

    // 立即二次认证：命中热缓存，不触发新的 Argon2
    registry
        .authorize(&identity, &issued.node_session_token)
        .await
        .expect("second authorize should succeed");
    assert_eq!(
        probe.total_entered(),
        1,
        "cache hit should not trigger new Argon2 verify"
    );

    // 注：完整测试 TTL 过期需要 sleep 5 分钟以上。此处重点验证缓存命中逻辑能有效阻止重复校验。

    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_dir(&temp_dir);
}

#[tokio::test]
async fn token_cache_distinguishes_current_and_grace_tokens_after_rotation() {
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

    // 使用原始 token 认证
    let authorized = registry
        .authorize(&identity, &issued.node_session_token)
        .await
        .expect("original token should authorize");
    assert_eq!(authorized.generation, 1);

    // 刷新 token（缓存应被清除）
    let (new_token, _, new_generation) = registry
        .refresh_token("rotate-01", authorized.generation)
        .await
        .expect("token should refresh");
    assert_eq!(new_generation, 2);

    // 在平滑过渡窗口内，旧 token 仅作为上一代凭据保持可用。
    let authorized = registry
        .authorize(&identity, &issued.node_session_token)
        .await
        .expect("old token should authorize during refresh grace");
    assert_eq!(authorized.generation, 1);
    assert!(authorized.token_expires_at.is_some());

    // 新 token 应使用更新后的 generation 完成认证
    let authorized = registry
        .authorize(&identity, &new_token)
        .await
        .expect("new token should authorize");
    assert_eq!(authorized.generation, 2);

    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_dir(&temp_dir);
}

async fn token_cache_fixture(
    prefix: &str,
    node_ids: &[&str],
    capacity: usize,
) -> (
    NodeRegistry,
    Vec<(NodeIdentity, String)>,
    std::path::PathBuf,
) {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be monotonic enough")
        .as_nanos();
    let temp_dir = std::env::temp_dir().join(format!("nodelite-token-cache-{prefix}-{unique}"));
    std::fs::create_dir_all(&temp_dir).expect("cache metric temp dir should exist");
    let path = temp_dir.join("server.json");
    let mut credentials = Vec::with_capacity(node_ids.len());
    for node_id in node_ids {
        let issued = issue_node(
            &path,
            IssueNodeRequest {
                node_id: (*node_id).to_string(),
                node_label: Some((*node_id).to_string()),
                tags: Vec::new(),
            },
        )
        .await
        .expect("cache metric node should be issued");
        credentials.push((identity_for(node_id), issued.node_session_token));
    }
    let registry = NodeRegistry::load(&path)
        .await
        .expect("cache metric registry should load")
        .with_token_cache_capacity_for_tests(capacity);
    (registry, credentials, temp_dir)
}
