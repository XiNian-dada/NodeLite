use super::*;

#[test]
fn registry_lock_drop_cleanup_swallows_panics_and_runs_both_steps() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    let cleanup_steps = Arc::new(AtomicUsize::new(0));
    let unlock_steps = Arc::clone(&cleanup_steps);
    let harden_steps = Arc::clone(&cleanup_steps);

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        release_registry_lock_with(
            move || {
                unlock_steps.fetch_or(0b01, Ordering::SeqCst);
                panic!("unlock panic");
            },
            move || {
                harden_steps.fetch_or(0b10, Ordering::SeqCst);
                panic!("harden panic");
            },
        );
    }));

    assert!(
        result.is_ok(),
        "cleanup helper should swallow internal panics"
    );
    assert_eq!(cleanup_steps.load(Ordering::SeqCst), 0b11);
}

#[tokio::test]
async fn registry_load_rejects_oversized_files_before_reading_json() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let temp_dir = std::env::temp_dir().join(format!("nodelite-registry-too-large-{unique}"));
    std::fs::create_dir_all(&temp_dir).expect("temp dir");
    let path = temp_dir.join("server.json");
    let file = std::fs::File::create(&path).expect("registry file should be created");
    file.set_len(MAX_REGISTRY_FILE_BYTES + 1)
        .expect("registry fixture should be expanded");

    reset_registry_file_read_count();
    let error = NodeRegistry::load(&path)
        .await
        .expect_err("oversized registry files should fail before JSON parsing");

    assert!(
        matches!(
            error,
            RegistryError::FileTooLarge {
                ref path,
                len,
                max_len,
            } if path.ends_with("server.json")
                && len == MAX_REGISTRY_FILE_BYTES + 1
                && max_len == MAX_REGISTRY_FILE_BYTES
        ),
        "unexpected error: {error:?}"
    );
    assert_eq!(
        registry_file_read_count(),
        0,
        "oversized registry files should be rejected before read_to_string"
    );

    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_dir(&temp_dir);
}

#[tokio::test]
async fn registry_load_missing_file_still_returns_empty_registry() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let temp_dir = std::env::temp_dir().join(format!("nodelite-registry-missing-file-{unique}"));
    std::fs::create_dir_all(&temp_dir).expect("temp dir");
    let path = temp_dir.join("server.json");

    let registry = NodeRegistry::load(&path)
        .await
        .expect("missing registry files should load as empty state");

    assert_eq!(registry.count().await, 0);

    let _ = std::fs::remove_dir(&temp_dir);
}

#[tokio::test]
async fn refresh_token_reports_missing_nodes_with_typed_error() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let temp_dir = std::env::temp_dir().join(format!("nodelite-registry-missing-node-{unique}"));
    std::fs::create_dir_all(&temp_dir).expect("temp dir");
    let path = temp_dir.join("server.json");
    std::fs::write(&path, "{\"nodes\":[],\"install_sessions\":[]}")
        .expect("empty registry should be written");
    let registry = NodeRegistry::load(&path)
        .await
        .expect("registry should load");

    let error = registry
        .refresh_token("missing-01")
        .await
        .expect_err("missing nodes should surface a typed error");
    assert!(matches!(error, RegistryError::NodeNotFound(ref node_id) if node_id == "missing-01"));

    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_dir(&temp_dir);
}
