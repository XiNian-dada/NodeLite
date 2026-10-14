//! Renewal regression tests cover both durable crash windows and explicit revocation.

use super::*;

struct Fixture {
    dir: std::path::PathBuf,
    path: std::path::PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!(
            "nodelite-renewal-{}",
            super::super::generate_token().expect("random fixture ID")
        ));
        std::fs::create_dir(&dir).expect("fixture directory");
        Self {
            path: dir.join("server.json"),
            dir,
        }
    }
    async fn issue(&self) -> super::super::IssueNodeResult {
        issue_node(
            &self.path,
            IssueNodeRequest {
                node_id: "renewal-01".into(),
                node_label: None,
                tags: Vec::new(),
            },
        )
        .await
        .expect("issue fixture node")
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[tokio::test]
async fn staging_preserves_current_token_and_both_credentials_survive_restart() {
    let fixture = Fixture::new();
    let original = fixture.issue().await;
    let registry = NodeRegistry::load(&fixture.path).await.expect("load");
    let candidate = registry
        .prepare_token_refresh("renewal-01", 1)
        .await
        .expect("stage");
    let disk = std::fs::read_to_string(&fixture.path).expect("disk registry");
    assert!(!disk.contains(&candidate.token));
    assert_eq!(
        registry
            .registered_node("renewal-01")
            .await
            .expect("node")
            .token_generation,
        1
    );
    drop(registry);
    let restarted = NodeRegistry::load(&fixture.path).await.expect("restart");
    let identity = identity_for("renewal-01");
    assert_eq!(
        restarted
            .authorize(&identity, &original.node_session_token)
            .await
            .expect("old remains valid")
            .generation,
        1
    );
    assert_eq!(
        restarted
            .authorize(&identity, &candidate.token)
            .await
            .expect("saved candidate remains valid")
            .generation,
        2
    );
    restarted
        .activate_saved_token("renewal-01", 2)
        .await
        .expect("recover lost ACK");
    assert_eq!(
        restarted
            .registered_node("renewal-01")
            .await
            .expect("node")
            .token_generation,
        2
    );
    assert!(
        restarted
            .commit_token_refresh("renewal-01", &candidate.refresh_id)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn manual_reissue_revokes_all_pending_tokens_and_stale_acknowledgements() {
    let fixture = Fixture::new();
    let original = fixture.issue().await;
    let registry = NodeRegistry::load(&fixture.path).await.expect("load");
    let candidate = registry
        .prepare_token_refresh("renewal-01", 1)
        .await
        .expect("stage");
    let reissued = fixture.issue().await;
    assert!(
        reissued.node.token_generation > candidate.generation,
        "operator reissue must not reuse an authenticated pending generation"
    );
    registry.reload().await.expect("reload");
    assert!(
        registry
            .session_token_status("renewal-01", &candidate.token, candidate.generation)
            .await
            .is_err()
    );
    let identity = identity_for("renewal-01");
    assert!(
        registry
            .authorize(&identity, &original.node_session_token)
            .await
            .is_err()
    );
    assert!(
        registry
            .authorize(&identity, &candidate.token)
            .await
            .is_err()
    );
    assert!(
        registry
            .commit_token_refresh("renewal-01", &candidate.refresh_id)
            .await
            .is_err()
    );
    registry
        .authorize(&identity, &reissued.node_session_token)
        .await
        .expect("new operator credential valid");
}

#[tokio::test]
async fn lost_responses_are_bounded_without_overwriting_earlier_saved_candidates() {
    let fixture = Fixture::new();
    let original = fixture.issue().await;
    let registry = NodeRegistry::load(&fixture.path).await.expect("load");
    let first = registry
        .prepare_token_refresh("renewal-01", 1)
        .await
        .expect("stage");
    for _ in 1..super::super::renewal::MAX_PENDING_TOKENS {
        registry
            .prepare_token_refresh("renewal-01", 1)
            .await
            .expect("bounded stage");
    }
    assert!(
        registry
            .prepare_token_refresh("renewal-01", 1)
            .await
            .is_err()
    );
    let identity = identity_for("renewal-01");
    registry
        .authorize(&identity, &original.node_session_token)
        .await
        .expect("original not retired");
    registry
        .authorize(&identity, &first.token)
        .await
        .expect("earlier candidate not overwritten");
    assert!(
        registry
            .commit_token_refresh("renewal-01", "bad ID")
            .await
            .is_err()
    );
    registry
        .commit_token_refresh("renewal-01", &first.refresh_id)
        .await
        .expect("confirmed candidate");
    assert!(
        registry
            .registered_node("renewal-01")
            .await
            .expect("node")
            .pending_tokens
            .is_empty()
    );
}
