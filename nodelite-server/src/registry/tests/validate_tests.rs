use super::*;
use proptest::prelude::*;

#[test]
fn validate_registered_node_rejects_oversized_tags() {
    let mut node = RegisteredNode {
        node_id: "hk-01".to_string(),
        node_label: "Hong Kong 01".to_string(),
        token_hash: "hash".to_string(),
        token_generation: 1,
        token: "secret-token".to_string(),
        tags: vec!["edge".to_string()],
        created_at: Utc::now(),
        token_expires_at: None,
    };
    node.tags = vec!["x".repeat(MAX_NODE_TAG_BYTES + 1)];

    let error = validate_registered_node(&node).expect_err("oversized tag should fail");
    assert!(error.to_string().contains("node.tags[0]"));
}

#[tokio::test]
async fn issue_node_rejects_excessive_tags() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let temp_dir = std::env::temp_dir().join(format!("nodelite-registry-tag-limit-test-{unique}"));
    std::fs::create_dir_all(&temp_dir).expect("temp dir");
    let path = temp_dir.join("server.json");

    let error = issue_node(
        &path,
        IssueNodeRequest {
            node_id: "hk-01".to_string(),
            node_label: Some("Hong Kong 01".to_string()),
            tags: (0..1000).map(|index| format!("tag-{index}")).collect(),
        },
    )
    .await
    .expect_err("too many tags should fail");

    assert!(error.to_string().contains("tags"));
    let _ = std::fs::remove_dir_all(&temp_dir);
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]

    #[test]
    fn normalize_string_list_returns_sorted_trimmed_deduped_values(
        values in proptest::collection::vec(".*", 0..32),
    ) {
        let normalized = super::super::normalize_string_list(values.clone());

        for value in &normalized {
            prop_assert!(!value.is_empty());
            prop_assert_eq!(value.trim(), value);
        }

        let mut sorted = normalized.clone();
        sorted.sort();
        prop_assert_eq!(normalized.as_slice(), sorted.as_slice());

        let mut deduped = normalized.clone();
        deduped.dedup();
        prop_assert_eq!(normalized.as_slice(), deduped.as_slice());

        for value in &normalized {
            prop_assert!(values.iter().any(|original| original.trim() == value));
        }
    }
}
