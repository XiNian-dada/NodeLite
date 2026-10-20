use std::fs;
#[cfg(target_os = "linux")]
use std::process::Command;

use anyhow::Result;
use nodelite_agent::config_io::{load_agent_config, update_token_in_config};

mod common;
use common::TempDir;

#[tokio::test]
async fn test_agent_config_load_and_token_update() -> Result<()> {
    // 唯一的临时目录由 RAII 守卫管理:即便断言 panic 也会被清理。
    let temp_dir = TempDir::new("nodelite-agent-config-test");
    let config_path = temp_dir.path().join("agent.toml");

    let initial_toml = r#"[agent]
node_id = "test-node-01"
node_label = "Test Node 01"
server = "ws://127.0.0.1:8080/ws"
token = "initial-secret-token"
report_interval_secs = 5
"#;

    fs::write(&config_path, initial_toml)?;

    // 1. Load config and verify fields
    let config = load_agent_config(&config_path).await?;
    assert_eq!(config.node_id, "test-node-01");
    assert_eq!(config.node_label, "Test Node 01");
    assert_eq!(config.server, "ws://127.0.0.1:8080/ws");
    assert_eq!(config.token, "initial-secret-token");
    assert_eq!(config.report_interval_secs, 5);

    // 2. Update token in the config file
    update_token_in_config(&config_path, "refreshed-secret-token").await?;

    // 3. Reload config and verify token was updated
    let updated_config = load_agent_config(&config_path).await?;
    assert_eq!(updated_config.token, "refreshed-secret-token");
    assert_eq!(updated_config.node_id, "test-node-01"); // Other fields remain unchanged

    // 清理由 `TempDir` 的 Drop 负责。
    Ok(())
}

#[cfg(target_os = "linux")]
#[test]
fn signed_update_preflight_rejects_slow_reports_without_touching_credentials() -> Result<()> {
    let dir = TempDir::new("nodelite-signed-interval");
    let path = dir.path().join("agent.toml");
    let preflight = include_str!("../../nodelite-updater/systemd/nodelite-agent-preflight.service");
    assert!(preflight.contains("NODELITE_UPDATE_HEALTH=1"));
    for seconds in [1, 5, 10, 15, 20, 30] {
        let config = format!(
            "[agent]\nnode_id = 'interval-test'\nnode_label = 'Interval'\nserver = 'ws://127.0.0.1:1/ws'\ntoken = 'private-test-credential'\nreport_interval_secs = {seconds}\n"
        );
        fs::write(&path, &config)?;
        let run = |signed, action| {
            Command::new(env!("CARGO_BIN_EXE_nodelite-agent"))
                .args([action, "--config"])
                .arg(&path)
                .env("NODELITE_UPDATE_HEALTH", signed)
                .env("RUST_LOG", "off")
                .output()
        };
        let sample = run("1", "--sample-once")?;
        assert_eq!(sample.status.success(), seconds <= 10);
        if seconds > 10 {
            let probe = run("1", "--check-credentials")?;
            assert!(!probe.status.success());
            let error = String::from_utf8_lossy(&probe.stderr);
            assert!(error.contains("report_interval_secs between 1 and 10"));
            assert!(!error.contains("private-test-credential"));
        }
        assert!(run("0", "--sample-once")?.status.success());
        assert_eq!(fs::read_to_string(&path)?, config);
    }
    Ok(())
}
