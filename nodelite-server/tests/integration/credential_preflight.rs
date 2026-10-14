//! The candidate binary must validate disk credentials without displacing the live session.

use super::*;
use crate::test_support::{TestNode, agent_binary_path};

struct ConfigFixture {
    dir: std::path::PathBuf,
    path: std::path::PathBuf,
}
impl ConfigFixture {
    fn new(server: &TestServer, node: &TestNode) -> Result<Self> {
        let dir = std::env::temp_dir().join(format!("nodelite-preflight-{}", node.node_id));
        crate::fs_security::create_private_dir_all(&dir)?;
        let path = dir.join("agent.toml");
        let content = format!(
            "[agent]\nnode_id = {:?}\nnode_label = {:?}\nserver = {:?}\ntoken = {:?}\n",
            node.node_id,
            node.node_label,
            format!("ws://{}/ws", server.addr),
            node.token
        );
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        use std::io::Write;
        options.open(&path)?.write_all(content.as_bytes())?;
        Ok(Self { dir, path })
    }
    async fn check(&self) -> Result<std::process::Output> {
        Ok(tokio::process::Command::new(agent_binary_path()?)
            .args(["--check-credentials", "--config"])
            .arg(&self.path)
            .env("RUST_LOG", "off")
            .output()
            .await?)
    }
}
impl Drop for ConfigFixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn credential_preflight_does_not_register_or_displace_the_live_agent() -> Result<()> {
    let server = TestServer::start_with_config(|config| {
        config.ws.max_total_connections = 1;
        config.ws.max_connections_per_ip = 1;
    })
    .await?;
    let node = server
        .issue_node("itest-preflight-01", "Preflight 01")
        .await?;
    let fixture = ConfigFixture::new(&server, &node)?;
    let result = fixture.check().await?;
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(server.overview().await?.online_nodes, 0);
    let mut live = TestAgent::connect(&server, &node).await?;
    live.send_fake_metrics(1).await?;
    server
        .wait_for_node_uptime(&node.node_id, 1, TEST_TIMEOUT)
        .await?;
    assert!(fixture.check().await?.status.success());
    live.send_fake_metrics(2).await?;
    assert!(
        server
            .wait_for_node_uptime(&node.node_id, 2, TEST_TIMEOUT)
            .await?
            .online
    );
    live.disconnect().await?;
    server.shutdown().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn invalid_saved_credentials_fail_preflight_without_interrupting_a_healthy_ram_session()
-> Result<()> {
    let server = TestServer::start().await?;
    let node = server
        .issue_node("itest-preflight-02", "Preflight 02")
        .await?;
    let mut live = TestAgent::connect(&server, &node).await?;
    let stale = TestNode {
        token: "stale-fixture-that-must-not-be-logged".to_string(),
        ..node.clone()
    };
    let fixture = ConfigFixture::new(&server, &stale)?;
    let result = fixture.check().await?;
    assert!(!result.status.success());
    assert!(!String::from_utf8_lossy(&result.stderr).contains(&stale.token));
    live.send_fake_metrics(3).await?;
    assert!(
        server
            .wait_for_node_uptime(&node.node_id, 3, TEST_TIMEOUT)
            .await?
            .online
    );
    live.disconnect().await?;
    server.shutdown().await
}
