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

#[tokio::test]
async fn refreshed_token_grace_survives_registry_restart_and_expires() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let temp_dir = std::env::temp_dir().join(format!("nodelite-refresh-grace-{unique}"));
    std::fs::create_dir_all(&temp_dir).expect("temp dir");
    let path = temp_dir.join("server.json");

    let issued = issue_node(
        &path,
        IssueNodeRequest {
            node_id: "grace-01".to_string(),
            node_label: Some("Grace 01".to_string()),
            tags: Vec::new(),
        },
    )
    .await
    .expect("node should be issued");
    let registry = NodeRegistry::load(&path)
        .await
        .expect("registry should load");
    let error = registry
        .refresh_token("grace-01", 0)
        .await
        .expect_err("stale generation must not rotate the current token");
    assert!(matches!(error, RegistryError::Unauthorized));
    let (new_token, _, new_generation) = registry
        .refresh_token("grace-01", 1)
        .await
        .expect("token should refresh");
    assert_eq!(new_generation, 2);
    drop(registry);

    let restarted = NodeRegistry::load(&path)
        .await
        .expect("registry should reload after restart");
    let identity = identity_for("grace-01");
    let previous = restarted
        .authorize(&identity, &issued.node_session_token)
        .await
        .expect("previous token should survive restart during grace");
    assert_eq!(previous.generation, 1);
    let grace_deadline = previous
        .token_expires_at
        .expect("previous token should expose its grace deadline");
    assert!(grace_deadline > Utc::now());
    assert!(restarted.is_token_current("grace-01", 1).await);
    restarted
        .authorize(&identity, &new_token)
        .await
        .expect("current token should survive restart");
    drop(restarted);

    let mut file: RegistryFile =
        serde_json::from_str(&std::fs::read_to_string(&path).expect("registry should be readable"))
            .expect("registry JSON should parse");
    let node = file.nodes.first_mut().expect("node should exist");
    assert!(!node.previous_token_hash.is_empty());
    assert_eq!(node.previous_token_generation, Some(1));
    node.previous_token_valid_until = Some(Utc::now() - ChronoDuration::seconds(1));
    std::fs::write(
        &path,
        serde_json::to_string_pretty(&file).expect("registry should serialize"),
    )
    .expect("registry should be writable");

    let expired = NodeRegistry::load(&path)
        .await
        .expect("expired grace metadata should remain loadable");
    assert!(
        expired
            .authorize(&identity, &issued.node_session_token)
            .await
            .is_err(),
        "previous token should fail after grace expires"
    );
    assert!(!expired.is_token_current("grace-01", 1).await);
    expired
        .authorize(&identity, &new_token)
        .await
        .expect("current token should remain valid after grace expires");

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
        previous_token_hash: String::new(),
        previous_token_generation: None,
        previous_token_valid_until: None,
        token: "secret".to_string(),
        tags: Vec::new(),
        created_at: expires_at - ChronoDuration::minutes(5),
        token_expires_at: Some(expires_at),
        service_expires_at: None,
        service_unlimited: false,
        renewal_price: None,
        traffic_limit_bytes: None,
        traffic_accounting: super::super::TrafficAccounting::Bidirectional,
        traffic_throttle_kbps: None,
        location_override_country: None,
        location_override_city: None,
        location_override_latitude_microdegrees: None,
        location_override_longitude_microdegrees: None,
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
