use super::*;
use proptest::prelude::*;

#[tokio::test]
async fn issued_tokens_default_to_thirty_day_expiry() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let temp_dir = std::env::temp_dir().join(format!("nodelite-registry-expiry-test-{unique}"));
    std::fs::create_dir_all(&temp_dir).expect("temp dir");
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

    let expires_at = issued
        .node
        .token_expires_at
        .expect("issued token should carry expiry");
    let remaining = expires_at - Utc::now();
    assert!(
        remaining >= ChronoDuration::days(29)
            && remaining <= ChronoDuration::days(30) + ChronoDuration::minutes(1),
        "expected about 30 days of remaining validity, got {remaining:?}",
    );

    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_dir(&temp_dir);
}

#[test]
fn expired_tokens_are_not_current_after_handshake() {
    let runtime = Runtime::new().expect("runtime should build");
    runtime.block_on(async {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be monotonic enough")
            .as_nanos();
        let temp_dir = std::env::temp_dir().join(format!("nodelite-expired-token-test-{unique}"));
        std::fs::create_dir_all(&temp_dir).expect("temp dir should exist");
        let path = temp_dir.join("server.json");
        let file = RegistryFile {
            version: 0,
            nodes: vec![legacy_node(
                "expired-01",
                "Expired 01",
                "secret",
                Some(Utc::now() - ChronoDuration::seconds(1)),
            )],
            install_sessions: Vec::new(),
        };
        std::fs::write(&path, serde_json::to_string_pretty(&file).expect("json"))
            .expect("registry should be written");
        let registry = NodeRegistry::load(&path)
            .await
            .expect("registry should load");

        let error = registry
            .authorize(&identity_for("expired-01"), "secret")
            .await
            .expect_err("expired token should not authorize");
        assert!(matches!(
            error,
            RegistryError::TokenExpired { ref node_id } if node_id == "expired-01"
        ));
        assert!(!registry.is_token_current("expired-01", 1).await);

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(&temp_dir);
    });
}

#[test]
fn token_is_expired_at_exact_expiry_moment() {
    let expires_at = Utc.with_ymd_and_hms(2026, 5, 18, 12, 0, 0).unwrap();
    let entry = RegisteredNode {
        node_id: "boundary-01".to_string(),
        node_label: "Boundary 01".to_string(),
        token_hash: "hash".to_string(),
        token_generation: 1,
        token: "secret".to_string(),
        tags: Vec::new(),
        created_at: expires_at - ChronoDuration::minutes(5),
        token_expires_at: Some(expires_at),
    };

    assert!(!token_is_unexpired(&entry, expires_at));
    assert!(token_is_unexpired(
        &entry,
        expires_at - ChronoDuration::nanoseconds(1),
    ));
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]

    #[test]
    fn generate_token_always_returns_lowercase_hex(_case in any::<u8>()) {
        let token = super::super::generate_token().expect("token generation should succeed");
        prop_assert_eq!(token.len(), 64);
        prop_assert!(
            token
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        );
    }
}
