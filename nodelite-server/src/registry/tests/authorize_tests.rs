use super::*;

#[test]
fn registry_authorizes_per_node_token_and_overrides_metadata() {
    let runtime = Runtime::new().expect("runtime should build");
    runtime.block_on(async {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be monotonic enough")
            .as_nanos();
        let temp_dir = std::env::temp_dir().join(format!("nodelite-registry-auth-test-{unique}"));
        std::fs::create_dir_all(&temp_dir).expect("temp dir should exist");
        let path = temp_dir.join("server.json");
        let mut node = legacy_node("osaka-01", "Osaka 01", "secret", None);
        node.tags = vec!["edge".to_string()];
        let file = RegistryFile {
            version: 0,
            nodes: vec![node],
            install_sessions: Vec::new(),
        };
        std::fs::write(&path, serde_json::to_string_pretty(&file).expect("json"))
            .expect("registry should be written");
        let registry = NodeRegistry::load(&path)
            .await
            .expect("registry should load");
        let identity = NodeIdentity {
            node_id: "osaka-01".to_string(),
            node_label: "Wrong".to_string(),
            hostname: "osaka-01.internal".to_string(),
            os: "Ubuntu".to_string(),
            kernel_version: None,
            cpu_model: None,
            cpu_cores: 2,
            agent_version: "0.1.0".to_string(),
            boot_time: None,
            tags: vec!["wrong".to_string()],
        };

        let authorized = registry
            .authorize(&identity, "secret")
            .await
            .expect("identity should authorize");
        assert_eq!(authorized.identity.node_label, "Osaka 01");
        assert_eq!(authorized.identity.tags, vec!["edge"]);
        assert_eq!(authorized.generation, 1);

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(&temp_dir);
    });
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn token_verify_limiter_caps_parallel_argon2_verifies() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be monotonic enough")
        .as_nanos();
    let temp_dir = std::env::temp_dir().join(format!("nodelite-token-verify-limit-{unique}"));
    std::fs::create_dir_all(&temp_dir).expect("temp dir should exist");
    let path = temp_dir.join("server.json");

    let issued = issue_node(
        &path,
        IssueNodeRequest {
            node_id: "storm-01".to_string(),
            node_label: Some("Storm 01".to_string()),
            tags: Vec::new(),
        },
    )
    .await
    .expect("node should be issued");
    let probe = Arc::new(TokenVerifyProbe::new(Duration::from_millis(75)));
    let registry = NodeRegistry::load(&path)
        .await
        .expect("registry should load")
        .with_token_verify_limit_for_tests(1)
        .with_token_verify_probe_for_tests(Arc::clone(&probe));
    let identity = identity_for("storm-01");

    let mut handles = Vec::new();
    for _ in 0..6 {
        let registry = registry.clone();
        let identity = identity.clone();
        let token = issued.node_session_token.clone();
        handles.push(tokio::spawn(async move {
            registry.authorize(&identity, &token).await
        }));
    }

    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let metrics = registry.token_verify_metrics();
            if metrics.active == 1 && metrics.waiting >= 1 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("token verify metrics should observe active and waiting requests");

    for result in futures::future::join_all(handles).await {
        let authorized = result
            .expect("authorize task should complete")
            .expect("token should authorize");
        assert_eq!(authorized.identity.node_id, "storm-01");
    }
    assert_eq!(probe.max_active(), 1);
    let metrics = registry.token_verify_metrics();
    assert_eq!(metrics.limit, 1);
    assert_eq!(metrics.active, 0);
    assert_eq!(metrics.waiting, 0);
    assert!(metrics.wait_seconds_total > 0.0);

    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_dir(&temp_dir);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn token_verify_limiter_honors_reconnect_storm_matrix_and_cache() {
    for limit in [2, 4, 8] {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be monotonic enough")
            .as_nanos();
        let temp_dir =
            std::env::temp_dir().join(format!("nodelite-token-verify-matrix-{limit}-{unique}"));
        std::fs::create_dir_all(&temp_dir).expect("temp dir should exist");
        let path = temp_dir.join("server.json");
        let mut credentials = Vec::new();
        for index in 0..8 {
            let node_id = format!("storm-{limit}-{index}");
            let issued = issue_node(
                &path,
                IssueNodeRequest {
                    node_id: node_id.clone(),
                    node_label: Some(node_id.clone()),
                    tags: Vec::new(),
                },
            )
            .await
            .expect("node should be issued");
            credentials.push((identity_for(&node_id), issued.node_session_token));
        }

        let probe = Arc::new(TokenVerifyProbe::new(Duration::from_millis(75)));
        let registry = NodeRegistry::load_with_token_verify_limit(&path, limit)
            .await
            .expect("registry should load")
            .with_token_verify_probe_for_tests(Arc::clone(&probe));

        authorize_storm(&registry, &credentials).await;
        assert_eq!(
            probe.max_active(),
            limit,
            "configured limit should cap the {limit}-way storm"
        );
        assert_eq!(probe.total_entered(), credentials.len());

        authorize_storm(&registry, &credentials).await;
        assert_eq!(
            probe.total_entered(),
            credentials.len(),
            "warm reconnects should preserve token cache behavior"
        );

        let metrics = registry.token_verify_metrics();
        assert_eq!(metrics.limit, limit as u64);
        assert_eq!(metrics.active, 0);
        assert_eq!(metrics.waiting, 0);
        assert!(metrics.wait_seconds_total > 0.0);

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(&temp_dir);
    }
}

async fn authorize_storm(registry: &NodeRegistry, credentials: &[(NodeIdentity, String)]) {
    let handles = credentials.iter().map(|(identity, token)| {
        let registry = registry.clone();
        let identity = identity.clone();
        let token = token.clone();
        tokio::spawn(async move { registry.authorize(&identity, &token).await })
    });
    for result in futures::future::join_all(handles).await {
        result
            .expect("authorize task should complete")
            .expect("token should authorize");
    }
}

#[test]
fn unenrolled_nodes_are_rejected() {
    let runtime = Runtime::new().expect("runtime should build");
    runtime.block_on(async {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be monotonic enough")
            .as_nanos();
        let temp_dir = std::env::temp_dir().join(format!("nodelite-registry-legacy-test-{unique}"));
        std::fs::create_dir_all(&temp_dir).expect("temp dir should exist");
        let path = temp_dir.join("server.json");
        std::fs::write(&path, "{\"nodes\":[]}").expect("empty registry should be written");

        let registry = NodeRegistry::load(&path)
            .await
            .expect("registry should load");
        let identity = NodeIdentity {
            node_id: "legacy-01".to_string(),
            node_label: "Legacy 01".to_string(),
            hostname: "legacy-01.internal".to_string(),
            os: "Ubuntu".to_string(),
            kernel_version: None,
            cpu_model: None,
            cpu_cores: 2,
            agent_version: "0.1.0".to_string(),
            boot_time: None,
            tags: Vec::new(),
        };

        let error = registry
            .authorize(&identity, "some-token")
            .await
            .expect_err("unenrolled node should be rejected");
        assert!(matches!(error, RegistryError::Unauthorized));

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(&temp_dir);
    });
}

#[test]
fn wrong_tokens_use_the_same_auth_error() {
    let runtime = Runtime::new().expect("runtime should build");
    runtime.block_on(async {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be monotonic enough")
            .as_nanos();
        let temp_dir =
            std::env::temp_dir().join(format!("nodelite-registry-auth-error-test-{unique}"));
        std::fs::create_dir_all(&temp_dir).expect("temp dir should exist");
        let path = temp_dir.join("server.json");
        let file = RegistryFile {
            version: 0,
            nodes: vec![legacy_node("osaka-01", "Osaka 01", "secret", None)],
            install_sessions: Vec::new(),
        };
        std::fs::write(&path, serde_json::to_string_pretty(&file).expect("json"))
            .expect("registry should be written");
        let registry = NodeRegistry::load(&path)
            .await
            .expect("registry should load");
        let identity = NodeIdentity {
            node_id: "osaka-01".to_string(),
            node_label: "Osaka 01".to_string(),
            hostname: "osaka-01.internal".to_string(),
            os: "Ubuntu".to_string(),
            kernel_version: None,
            cpu_model: None,
            cpu_cores: 2,
            agent_version: "0.1.0".to_string(),
            boot_time: None,
            tags: Vec::new(),
        };

        let error = registry
            .authorize(&identity, "wrong-secret")
            .await
            .expect_err("wrong token should be rejected");
        assert!(matches!(error, RegistryError::Unauthorized));

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(&temp_dir);
    });
}
