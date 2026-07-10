use nodelite_proto::{MAX_NODE_IDENTITY_TEXT_BYTES, MAX_NODE_TAG_BYTES, MAX_NODE_TAGS};

use super::*;

#[test]
fn load_hashes_legacy_plaintext_tokens_and_persists_migration() {
    let runtime = Runtime::new().expect("runtime should build");
    runtime.block_on(async {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be monotonic enough")
            .as_nanos();
        let temp_dir =
            std::env::temp_dir().join(format!("nodelite-registry-migration-test-{unique}"));
        std::fs::create_dir_all(&temp_dir).expect("temp dir should exist");
        let path = temp_dir.join("server.json");
        let file = RegistryFile {
            version: 0,
            nodes: vec![legacy_node("legacy-01", "Legacy 01", "legacy-secret", None)],
            install_sessions: Vec::new(),
        };
        std::fs::write(&path, serde_json::to_string_pretty(&file).expect("json"))
            .expect("registry should be written");

        let registry = NodeRegistry::load(&path)
            .await
            .expect("registry should load");
        let authorized = registry
            .authorize(&identity_for("legacy-01"), "legacy-secret")
            .await
            .expect("legacy token should still authorize after migration");
        assert_eq!(authorized.generation, 1);

        let stored = std::fs::read_to_string(&path).expect("registry should be readable");
        assert!(!stored.contains("legacy-secret"));
        let parsed: RegistryFile =
            serde_json::from_str(&stored).expect("stored registry should parse");
        assert_eq!(parsed.nodes.len(), 1);
        assert!(parsed.nodes[0].token.is_empty());
        assert!(parsed.nodes[0].token_hash.starts_with("$argon2id$"));
        assert!(verify_token("legacy-secret", &parsed.nodes[0].token_hash));
        assert_eq!(parsed.nodes[0].token_generation, 1);
        assert_eq!(parsed.version, 1);

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(&temp_dir);
    });
}

#[test]
fn load_sanitizes_legacy_display_metadata_and_persists_once() {
    let runtime = Runtime::new().expect("runtime should build");
    runtime.block_on(async {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be monotonic enough")
            .as_nanos();
        let temp_dir = std::env::temp_dir().join(format!("nodelite-metadata-migration-{unique}"));
        std::fs::create_dir_all(&temp_dir).expect("temp dir should exist");
        let path = temp_dir.join("server.json");
        let mut node = legacy_node(
            "legacy-display-01",
            &format!("  Hong\nKong {}  ", "界".repeat(100)),
            "legacy-secret",
            None,
        );
        node.tags = (0..(MAX_NODE_TAGS + 5))
            .map(|index| format!(" tag-{index:02}\n{} ", "界".repeat(MAX_NODE_TAG_BYTES)))
            .chain([" edge ".to_string(), "edge".to_string(), "\0".to_string()])
            .collect();
        let empty_label = legacy_node("legacy-display-02", "\n\r\t", "second-secret", None);
        let file = RegistryFile {
            version: 7,
            nodes: vec![node, empty_label],
            install_sessions: Vec::new(),
        };
        std::fs::write(&path, serde_json::to_string_pretty(&file).expect("json"))
            .expect("registry should be written");

        let registry = NodeRegistry::load(&path)
            .await
            .expect("legacy display metadata should migrate during startup");
        let nodes = registry.list_registered_nodes().await;
        assert_eq!(nodes.len(), 2);
        let fallback = nodes
            .iter()
            .find(|node| node.node_id == "legacy-display-02")
            .expect("fallback node should exist");
        assert_eq!(fallback.node_label, "legacy-display-02");
        for node in &nodes {
            assert!(!node.node_label.is_empty());
            assert!(node.node_label.len() <= MAX_NODE_IDENTITY_TEXT_BYTES);
            assert!(!node.node_label.chars().any(char::is_control));
            assert!(node.tags.len() <= MAX_NODE_TAGS);
            assert!(node.tags.iter().all(|tag| {
                !tag.is_empty()
                    && tag.len() <= MAX_NODE_TAG_BYTES
                    && !tag.chars().any(char::is_control)
            }));
        }
        drop(registry);

        let stored = std::fs::read_to_string(&path).expect("registry should be readable");
        let parsed: RegistryFile = serde_json::from_str(&stored).expect("stored JSON should parse");
        assert_eq!(parsed.version, 8);
        NodeRegistry::load(&path)
            .await
            .expect("migrated registry should load without another rewrite");
        let reloaded: RegistryFile = serde_json::from_str(
            &std::fs::read_to_string(&path).expect("registry should remain readable"),
        )
        .expect("reloaded JSON should parse");
        assert_eq!(reloaded.version, 8);

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(&temp_dir);
    });
}
