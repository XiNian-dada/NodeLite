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
