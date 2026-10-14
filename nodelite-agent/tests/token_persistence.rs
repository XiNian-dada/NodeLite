//! Regression coverage for private, failure-safe token persistence.

use std::fs;

use nodelite_agent::config_io::{TokenPersistenceError, update_token_in_config};

mod common;
use common::TempDir;

#[tokio::test]
async fn persistence_preserves_original_on_invalid_config_without_leaking_secrets() {
    let temp = TempDir::new("nodelite-token-invalid");
    let path = temp.path().join("agent.toml");
    let content = "[agent]\ntoken = \"fixture-private-value\n";
    fs::write(&path, content).expect("write invalid fixture");
    let error = update_token_in_config(&path, "new-fixture-value")
        .await
        .expect_err("invalid config must fail");
    assert!(matches!(error, TokenPersistenceError::InvalidConfig));
    assert!(!format!("{error:?}").contains("fixture-private-value"));
    assert_eq!(fs::read_to_string(path).expect("read original"), content);
    assert_eq!(fs::read_dir(temp.path()).expect("list fixture").count(), 1);
}

#[tokio::test]
async fn persistence_is_private_and_leaves_no_temporary_credentials() {
    let temp = TempDir::new("nodelite-token-durable");
    let path = temp.path().join("agent.toml");
    fs::write(&path, "[agent]\ntoken = \"old-fixture\" # preserved\n").expect("write fixture");
    update_token_in_config(&path, "new-fixture")
        .await
        .expect("persist token");
    assert_eq!(
        fs::read_to_string(&path).expect("read updated"),
        "[agent]\ntoken = \"new-fixture\" # preserved\n"
    );
    assert_eq!(fs::read_dir(temp.path()).expect("list fixture").count(), 1);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(path).expect("metadata").permissions().mode() & 0o777,
            0o600
        );
    }
}
