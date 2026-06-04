use super::*;

#[test]
fn agent_server_url_uses_wss_for_https() {
    let url = build_agent_server_url("https://monitor.example.com").expect("url should build");
    assert_eq!(url, "wss://monitor.example.com/ws");
}

#[test]
fn issue_node_persists_registry_and_renders_install_command() {
    let runtime = Runtime::new().expect("runtime should build");
    runtime.block_on(async {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be monotonic enough")
            .as_nanos();
        let temp_dir = std::env::temp_dir().join(format!("nodelite-registry-test-{unique}"));
        std::fs::create_dir_all(&temp_dir).expect("temp dir should exist");
        let path = temp_dir.join("server.json");
        let issued = issue_node(
            &path,
            IssueNodeRequest {
                node_id: "hk-01".to_string(),
                node_label: Some("Hong Kong 01".to_string()),
                tags: vec!["edge".to_string(), "apac".to_string()],
            },
        )
        .await
        .expect("node should be issued");
        assert!(issued.created);

        let stored = std::fs::read_to_string(&path).expect("registry should be stored");
        let parsed: RegistryFile =
            serde_json::from_str(&stored).expect("stored registry should parse");
        assert_eq!(parsed.version, 1);
        assert_eq!(parsed.nodes.len(), 1);
        assert_eq!(parsed.install_sessions.len(), 1);
        assert!(parsed.nodes[0].token.is_empty());
        assert!(parsed.nodes[0].token_hash.starts_with("$argon2id$"));
        assert_ne!(parsed.nodes[0].token_hash, issued.node_session_token);
        assert_eq!(parsed.nodes[0].token_generation, 1);
        assert_eq!(parsed.install_sessions[0].token, issued.install_token);
        assert_eq!(
            parsed.install_sessions[0].node_session_token,
            issued.node_session_token
        );

        let command = render_install_command(
            "https://monitor.example.com",
            &issued.install_token,
            "https://github.com/XiNian-dada/NodeLite/releases/latest/download",
        )
        .expect("install command should render");
        assert!(command.contains("--bootstrap-url"));
        assert!(command.contains("/install-agent.sh"));
        assert!(command.contains("NODELITE_AGENT_INSTALL_TOKEN="));
        assert!(command.contains(&issued.install_token));
        assert!(!command.contains(&issued.node_session_token));

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(&temp_dir);
    });
}

#[test]
fn install_tokens_are_one_time_use() {
    let runtime = Runtime::new().expect("runtime should build");
    runtime.block_on(async {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be monotonic enough")
            .as_nanos();
        let temp_dir = std::env::temp_dir().join(format!("nodelite-install-token-test-{unique}"));
        std::fs::create_dir_all(&temp_dir).expect("temp dir should exist");
        let path = temp_dir.join("server.json");

        let issued = issue_node(
            &path,
            IssueNodeRequest {
                node_id: "hk-01".to_string(),
                node_label: Some("Hong Kong 01".to_string()),
                tags: Vec::new(),
            },
        )
        .await
        .expect("node should be issued");
        let registry = NodeRegistry::load(&path)
            .await
            .expect("registry should load");

        let consumed = registry
            .consume_install_token(&issued.install_token)
            .await
            .expect("install token should be consumable")
            .expect("install token should resolve to a node");
        assert_eq!(consumed.node.node_id, issued.node.node_id);
        assert_eq!(consumed.node_session_token, issued.node_session_token);
        assert!(
            registry
                .consume_install_token(&issued.install_token)
                .await
                .expect("second install token lookup should succeed")
                .is_none()
        );
        let stored = std::fs::read_to_string(&path).expect("registry should be readable");
        assert!(!stored.contains(&issued.node_session_token));

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(&temp_dir);
    });
}

#[cfg(unix)]
#[test]
fn issued_registry_file_is_mode_600() {
    use std::os::unix::fs::PermissionsExt;

    let runtime = Runtime::new().expect("runtime should build");
    runtime.block_on(async {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be monotonic enough")
            .as_nanos();
        let temp_dir = std::env::temp_dir().join(format!("nodelite-registry-mode-test-{unique}"));
        std::fs::create_dir_all(&temp_dir).expect("temp dir should exist");
        let config_dir = temp_dir.join("config");
        let path = config_dir.join("server.json");

        issue_node(
            &path,
            IssueNodeRequest {
                node_id: "hk-01".to_string(),
                node_label: Some("Hong Kong 01".to_string()),
                tags: Vec::new(),
            },
        )
        .await
        .expect("node should be issued");

        let dir_mode = std::fs::metadata(&config_dir)
            .expect("config dir should exist")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(dir_mode, 0o700);

        let mode = std::fs::metadata(&path)
            .expect("metadata should exist")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(&config_dir);
        let _ = std::fs::remove_dir(&temp_dir);
    });
}

#[test]
fn github_release_base_url_uses_latest_download_path() {
    let release_url = build_github_release_base_url("https://github.com/XiNian-dada/NodeLite.git")
        .expect("release url should build");
    assert_eq!(
        release_url,
        "https://github.com/XiNian-dada/NodeLite/releases/latest/download"
    );
}

#[test]
fn default_release_base_url_points_at_github_latest_download() {
    let release_url = default_agent_release_base_url().expect("default release url should build");
    assert_eq!(
        release_url,
        "https://github.com/XiNian-dada/NodeLite/releases/latest/download"
    );
}

#[tokio::test]
async fn concurrent_issue_node_preserves_all_nodes() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let temp_dir = std::env::temp_dir().join(format!("nodelite-registry-concurrent-test-{unique}"));
    std::fs::create_dir_all(&temp_dir).expect("temp dir");
    let path = temp_dir.join("server.json");

    // 并发 issue 10 个不同节点,验证乐观版本提交 + 唯一 tmp 文件名能保证全部落盘。
    let mut handles = Vec::new();
    for i in 0..10 {
        let path = path.clone();
        let handle = tokio::spawn(async move {
            issue_node(
                &path,
                IssueNodeRequest {
                    node_id: format!("node-{i:02}"),
                    node_label: Some(format!("Node {i:02}")),
                    tags: Vec::new(),
                },
            )
            .await
            .expect("issue should succeed")
        });
        handles.push(handle);
    }

    let results = futures::future::join_all(handles).await;
    assert_eq!(results.len(), 10, "all tasks should complete");

    let registry = NodeRegistry::load(&path).await.expect("load");
    let stored = std::fs::read_to_string(&path).expect("registry should be readable");
    let parsed: RegistryFile = serde_json::from_str(&stored).expect("registry should parse");
    let node_ids: Vec<_> = registry
        .state
        .read()
        .await
        .entries
        .keys()
        .cloned()
        .collect();
    assert_eq!(
        node_ids.len(),
        10,
        "all 10 nodes should be present in registry"
    );
    assert_eq!(
        parsed.version, 10,
        "each successful mutation should advance version"
    );

    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_dir(&temp_dir);
}
