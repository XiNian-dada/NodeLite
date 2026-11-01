//! 注册表文件被删除或回退时的内存/磁盘分叉检测测试模块。

use std::path::{Path, PathBuf};

use super::*;

fn temp_registry_path(label: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be monotonic enough")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("nodelite-registry-{label}-{unique}"));
    std::fs::create_dir_all(&dir).expect("temp dir should exist");
    dir.join("server.json")
}

fn request(node_id: &str) -> IssueNodeRequest {
    IssueNodeRequest {
        node_id: node_id.to_string(),
        node_label: None,
        tags: Vec::new(),
    }
}

fn disk_version(path: &Path) -> u64 {
    let content = std::fs::read_to_string(path).expect("registry file should be readable");
    serde_json::from_str::<RegistryFile>(&content)
        .expect("registry file should parse")
        .version
}

fn cleanup(path: &Path) {
    let _ = std::fs::remove_file(path);
    if let Some(dir) = path.parent() {
        let _ = std::fs::remove_dir(dir);
    }
}

#[test]
fn deleted_registry_file_is_reported_and_blocks_writes() {
    let runtime = Runtime::new().expect("runtime should build");
    runtime.block_on(async {
        let path = temp_registry_path("deleted");
        issue_node(&path, request("hk-01"))
            .await
            .expect("issue hk-01");
        issue_node(&path, request("hk-02"))
            .await
            .expect("issue hk-02");
        let registry = NodeRegistry::load(&path)
            .await
            .expect("registry should load");
        let snapshot = std::fs::read(&path).expect("registry should be readable");

        std::fs::remove_file(&path).expect("registry file should be removable");

        for result in [
            registry.reload().await,
            registry.reload_if_file_changed().await,
        ] {
            assert!(
                matches!(
                    result,
                    Err(RegistryError::Diverged {
                        disk_version: None,
                        ..
                    })
                ),
                "missing file must not be accepted as an empty registry: {result:?}",
            );
        }
        assert_eq!(registry.count().await, 2, "in-memory nodes must be kept");

        let issued = registry.issue_node(request("hk-03")).await;
        assert!(
            matches!(issued, Err(RegistryError::Diverged { .. })),
            "{issued:?}"
        );
        let removed = registry.remove_node("hk-01").await;
        assert!(
            matches!(removed, Err(RegistryError::Diverged { .. })),
            "{removed:?}"
        );
        assert!(
            !path.exists(),
            "refused writes must not recreate a partial registry"
        );

        std::fs::write(&path, snapshot).expect("registry should be restorable");
        registry
            .reload()
            .await
            .expect("restored registry should reload");
        registry
            .issue_node(request("hk-03"))
            .await
            .expect("writes should resume after restore");
        registry.reload().await.expect("reload after write");
        assert_eq!(registry.count().await, 3);

        cleanup(&path);
    });
}

#[test]
fn rolled_back_registry_file_is_reported_and_blocks_writes() {
    let runtime = Runtime::new().expect("runtime should build");
    runtime.block_on(async {
        let path = temp_registry_path("rollback");
        issue_node(&path, request("hk-01"))
            .await
            .expect("issue hk-01");
        let backup = std::fs::read(&path).expect("backup should be readable");
        issue_node(&path, request("hk-02"))
            .await
            .expect("issue hk-02");
        let registry = NodeRegistry::load(&path)
            .await
            .expect("registry should load");
        let current_version = disk_version(&path);

        std::fs::write(&path, &backup).expect("old backup should overwrite registry");
        let old_version = disk_version(&path);
        assert!(old_version < current_version);

        for result in [
            registry.reload_if_file_changed().await,
            registry.reload().await,
        ] {
            assert!(
                matches!(
                    result,
                    Err(RegistryError::Diverged { disk_version: Some(v), memory_version, .. })
                        if v == old_version && memory_version == current_version
                ),
                "rolled back file must be reported: {result:?}",
            );
        }
        assert_eq!(registry.count().await, 2, "in-memory nodes must be kept");

        let issued = registry.issue_node(request("hk-03")).await;
        assert!(
            matches!(issued, Err(RegistryError::Diverged { .. })),
            "{issued:?}"
        );
        let relocated = registry
            .update_location_override("hk-01", None, None, None, None)
            .await;
        assert!(
            matches!(relocated, Err(RegistryError::Diverged { .. })),
            "{relocated:?}"
        );
        assert_eq!(
            disk_version(&path),
            old_version,
            "rolled back file stays untouched"
        );

        cleanup(&path);
    });
}

#[test]
fn first_deploy_without_registry_file_stays_writable() {
    let runtime = Runtime::new().expect("runtime should build");
    runtime.block_on(async {
        let path = temp_registry_path("fresh");
        let registry = NodeRegistry::load(&path)
            .await
            .expect("empty registry should load");
        assert!(
            !registry
                .reload()
                .await
                .expect("missing file is fine before any commit")
        );
        registry
            .issue_node(request("hk-01"))
            .await
            .expect("first issue should succeed");
        assert!(
            registry
                .reload()
                .await
                .expect("reload should pick up first node")
        );
        assert_eq!(registry.count().await, 1);
        cleanup(&path);
    });
}

fn expire_install_sessions_on_disk(path: &Path, version_bump: u64) {
    let content = std::fs::read_to_string(path).expect("registry file should be readable");
    let mut value: serde_json::Value =
        serde_json::from_str(&content).expect("registry file should parse");
    let past = serde_json::to_value(Utc::now() - ChronoDuration::hours(1))
        .expect("timestamp should serialize");
    for session in value["install_sessions"]
        .as_array_mut()
        .expect("install sessions should be present")
    {
        session["expires_at"] = past.clone();
    }
    let version = value["version"]
        .as_u64()
        .expect("version should be numeric");
    value["version"] = (version + version_bump).into();
    std::fs::write(path, value.to_string()).expect("registry file should be writable");
}

fn disk_install_sessions(path: &Path) -> usize {
    let content = std::fs::read_to_string(path).expect("registry file should be readable");
    serde_json::from_str::<RegistryFile>(&content)
        .expect("registry file should parse")
        .install_sessions
        .len()
}

#[test]
fn expired_install_sessions_pruned_at_load_are_persisted() {
    let runtime = Runtime::new().expect("runtime should build");
    runtime.block_on(async {
        let path = temp_registry_path("prune-load");
        issue_node(&path, request("hk-01"))
            .await
            .expect("issue hk-01");
        expire_install_sessions_on_disk(&path, 0);
        let version = disk_version(&path);

        NodeRegistry::load(&path)
            .await
            .expect("registry should load");

        assert_eq!(
            disk_install_sessions(&path),
            0,
            "expired plaintext must leave disk"
        );
        assert_eq!(disk_version(&path), version + 1);
        cleanup(&path);
    });
}

#[test]
fn expired_install_sessions_pruned_at_reload_are_persisted() {
    let runtime = Runtime::new().expect("runtime should build");
    runtime.block_on(async {
        let path = temp_registry_path("prune-reload");
        let registry = NodeRegistry::load(&path)
            .await
            .expect("empty registry should load");
        issue_node(&path, request("hk-01"))
            .await
            .expect("issue hk-01");
        expire_install_sessions_on_disk(&path, 1);

        assert!(registry.reload().await.expect("reload should succeed"));

        assert_eq!(
            disk_install_sessions(&path),
            0,
            "expired plaintext must leave disk"
        );
        assert_eq!(registry.count().await, 1);
        cleanup(&path);
    });
}

#[test]
fn rolled_back_file_with_expired_sessions_is_not_rewritten() {
    let runtime = Runtime::new().expect("runtime should build");
    runtime.block_on(async {
        let path = temp_registry_path("prune-rollback");
        issue_node(&path, request("hk-01"))
            .await
            .expect("issue hk-01");
        let backup = std::fs::read(&path).expect("backup should be readable");
        issue_node(&path, request("hk-02"))
            .await
            .expect("issue hk-02");
        issue_node(&path, request("hk-03"))
            .await
            .expect("issue hk-03");
        let registry = NodeRegistry::load(&path)
            .await
            .expect("registry should load");

        std::fs::write(&path, &backup).expect("old backup should overwrite registry");
        expire_install_sessions_on_disk(&path, 0);
        let old_version = disk_version(&path);

        let result = registry.reload().await;
        assert!(
            matches!(result, Err(RegistryError::Diverged { .. })),
            "{result:?}"
        );
        assert_eq!(
            disk_version(&path),
            old_version,
            "guard must block the prune write"
        );
        assert_eq!(registry.count().await, 3);
        cleanup(&path);
    });
}

fn edit_disk(path: &Path, edit: impl FnOnce(&mut serde_json::Value)) {
    let content = std::fs::read_to_string(path).expect("registry file should be readable");
    let mut value: serde_json::Value =
        serde_json::from_str(&content).expect("registry file should parse");
    edit(&mut value);
    std::fs::write(path, value.to_string()).expect("registry file should be writable");
}

#[test]
fn versionless_populated_registry_refuses_recreating_a_deleted_file() {
    let runtime = Runtime::new().expect("runtime should build");
    runtime.block_on(async {
        let path = temp_registry_path("versionless");
        issue_node(&path, request("hk-01"))
            .await
            .expect("issue hk-01");
        edit_disk(&path, |value| {
            value
                .as_object_mut()
                .expect("registry file should be an object")
                .remove("version");
        });
        let registry = NodeRegistry::load(&path)
            .await
            .expect("registry should load");
        assert_eq!(disk_version(&path), 0);

        std::fs::remove_file(&path).expect("registry file should be removable");

        let reloaded = registry.reload().await;
        assert!(
            matches!(reloaded, Err(RegistryError::Diverged { .. })),
            "{reloaded:?}"
        );
        let issued = registry.issue_node(request("hk-02")).await;
        assert!(
            matches!(issued, Err(RegistryError::Diverged { .. })),
            "{issued:?}"
        );
        assert!(!path.exists(), "refused write must not recreate the file");
        assert_eq!(registry.count().await, 1);
        cleanup(&path);
    });
}

#[test]
fn rolled_back_file_needing_migration_is_not_rewritten_or_accepted() {
    let runtime = Runtime::new().expect("runtime should build");
    runtime.block_on(async {
        let path = temp_registry_path("migrate-rollback");
        issue_node(&path, request("hk-01"))
            .await
            .expect("issue hk-01");
        let backup = std::fs::read(&path).expect("backup should be readable");
        issue_node(&path, request("hk-02"))
            .await
            .expect("issue hk-02");
        let registry = NodeRegistry::load(&path)
            .await
            .expect("registry should load");
        let memory_version = disk_version(&path);

        // 备份只落后一个版本：若迁移先写，版本恰好追平内存，旧内容就会被接受。
        std::fs::write(&path, &backup).expect("old backup should overwrite registry");
        edit_disk(&path, |value| {
            value["nodes"][0]["token"] = "legacy-plaintext-token-0123456789abcdef".into();
            value["nodes"][0]["token_hash"] = "".into();
        });
        assert_eq!(disk_version(&path) + 1, memory_version);
        let rolled_back = std::fs::read(&path).expect("file should be readable");

        for _ in 0..2 {
            let result = registry.reload().await;
            assert!(
                matches!(result, Err(RegistryError::Diverged { .. })),
                "{result:?}"
            );
        }
        assert_eq!(
            std::fs::read(&path).expect("file should be readable"),
            rolled_back,
            "reload must not rewrite a rolled back file",
        );
        assert_eq!(registry.count().await, 2);
        cleanup(&path);
    });
}
