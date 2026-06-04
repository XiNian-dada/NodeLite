use super::*;

#[test]
fn registry_reload_picks_up_rotated_tokens() {
    let runtime = Runtime::new().expect("runtime should build");
    runtime.block_on(async {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be monotonic enough")
            .as_nanos();
        let temp_dir = std::env::temp_dir().join(format!("nodelite-registry-reload-test-{unique}"));
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
        let old_token = issued.node_session_token.clone();
        let registry = NodeRegistry::load(&path)
            .await
            .expect("registry should load");
        let identity = identity_for("hk-01");
        let old_authorized = registry
            .authorize(&identity, &old_token)
            .await
            .expect("old token should authorize before rotation");
        assert!(
            registry
                .is_token_current("hk-01", old_authorized.generation)
                .await
        );

        let rotated = issue_node(
            &path,
            IssueNodeRequest {
                node_id: "hk-01".to_string(),
                node_label: Some("Hong Kong 01".to_string()),
                tags: Vec::new(),
            },
        )
        .await
        .expect("node token should rotate");
        assert!(registry.reload().await.expect("reload should succeed"));
        assert!(
            !registry
                .is_token_current("hk-01", old_authorized.generation)
                .await
        );
        assert!(
            registry.authorize(&identity, &old_token).await.is_err(),
            "old plaintext token should no longer authorize"
        );
        let rotated_authorized = registry
            .authorize(&identity, &rotated.node_session_token)
            .await
            .expect("rotated token should authorize");
        assert!(
            registry
                .is_token_current("hk-01", rotated_authorized.generation)
                .await
        );

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(&temp_dir);
    });
}

#[test]
fn registry_reload_skips_full_read_when_metadata_is_unchanged() {
    let runtime = Runtime::new().expect("runtime should build");
    runtime.block_on(async {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be monotonic enough")
            .as_nanos();
        let temp_dir =
            std::env::temp_dir().join(format!("nodelite-registry-reload-skip-test-{unique}"));
        std::fs::create_dir_all(&temp_dir).expect("temp dir should exist");
        let path = temp_dir.join("server.json");

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
        let registry = NodeRegistry::load(&path)
            .await
            .expect("registry should load");

        reset_registry_file_read_count();
        assert!(
            !registry
                .reload_if_file_changed()
                .await
                .expect("unchanged registry should skip reload")
        );
        assert_eq!(
            registry_file_read_count(),
            0,
            "unchanged metadata should avoid full JSON reads",
        );

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(&temp_dir);
    });
}

#[test]
fn registry_reload_if_file_changed_picks_up_external_changes() {
    let runtime = Runtime::new().expect("runtime should build");
    runtime.block_on(async {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be monotonic enough")
            .as_nanos();
        let temp_dir =
            std::env::temp_dir().join(format!("nodelite-registry-reload-change-test-{unique}"));
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
        let identity = identity_for("hk-01");
        let old_authorized = registry
            .authorize(&identity, &issued.node_session_token)
            .await
            .expect("old token should authorize before rotation");

        let rotated = issue_node(
            &path,
            IssueNodeRequest {
                node_id: "hk-01".to_string(),
                node_label: Some("Hong Kong 01".to_string()),
                tags: Vec::new(),
            },
        )
        .await
        .expect("node token should rotate");

        reset_registry_file_read_count();
        assert!(
            registry
                .reload_if_file_changed()
                .await
                .expect("changed registry should reload")
        );
        assert!(
            registry_file_read_count() > 0,
            "changed metadata should trigger a full JSON read",
        );
        assert!(
            !registry
                .is_token_current("hk-01", old_authorized.generation)
                .await
        );
        registry
            .authorize(&identity, &rotated.node_session_token)
            .await
            .expect("rotated token should authorize after reload");

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(&temp_dir);
    });
}
