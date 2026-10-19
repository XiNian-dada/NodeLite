//! Explicitly ignored VM-only tests use actual Agent ELF and production WS/registry handlers.

use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use anyhow::{Context, Result, bail, ensure};
use chrono::Utc;
use serde_json::{Value, json};
use tokio::io::AsyncWriteExt;
use tokio::time::{sleep, timeout};
use tokio_util::sync::CancellationToken;

use crate::registry::NodeRegistry;
use crate::test_support::{TestNode, TestServer};

#[path = "real_proxy.rs"]
mod proxy;

const SCRIPT: &str = "/home/lab/agent-update-isolation/real_guest.py";
const CONFIG: &str = "/var/lib/nlup-lab/agent/agent.toml";
const CHECKPOINT: &str = "/var/lib/nlup-lab/private/checkpoint.json";

fn version(sequence: u8) -> String {
    let (key, fallback) = if sequence == 1 {
        ("NODELITE_LAB_OLD_VERSION", "lab-old")
    } else {
        ("NODELITE_LAB_NEW_VERSION", "lab-new")
    };
    std::env::var(key).unwrap_or_else(|_| fallback.to_string())
}

async fn lab(args: &[&str], input: Option<&str>) -> Result<Value> {
    let mut child = tokio::process::Command::new("python3")
        .arg(SCRIPT)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()?;
    if let Some(mut stream) = child.stdin.take() {
        stream.write_all(input.unwrap_or("").as_bytes()).await?;
    }
    let output = timeout(Duration::from_secs(25), child.wait_with_output()).await??;
    ensure!(
        output.status.success(),
        "VM control action failed: {}",
        args[0]
    );
    Ok(serde_json::from_slice(&output.stdout)?)
}

fn saved_token() -> Result<String> {
    nodelite_proto::parse_agent_config(&std::fs::read_to_string(CONFIG)?)
        .map(|config| config.token)
        .map_err(|_| anyhow::anyhow!("saved test config failed validation"))
}

async fn online(server: &TestServer, node: &TestNode, version: &str) -> Result<()> {
    timeout(Duration::from_secs(20), async {
        loop {
            let process = lab(&["probe"], None).await?;
            let heartbeat = std::fs::read("/var/lib/nlup-lab/status/heartbeat.json")
                .ok()
                .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok());
            let observed = heartbeat.is_some_and(|health| {
                health["pid"] == process["pid"] && health["digest"] == process["digest"]
            });
            if let Ok(status) = server.node_status(&node.node_id).await
                && observed
                && status.online
                && status.identity.agent_version == version
                && status.snapshot.is_some_and(|snapshot| {
                    (Utc::now() - snapshot.collected_at).num_milliseconds() < 1500
                })
            {
                return Ok::<_, anyhow::Error>(());
            }
            sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .context("real Agent did not authenticate/report")?
}

async fn observe(
    server: &TestServer,
    node: &TestNode,
    identities: &Value,
    stop: CancellationToken,
) -> Result<()> {
    let mut invocation = Value::Null;
    let mut first_seen = Utc::now();
    while !stop.is_cancelled() {
        let process = lab(&["probe"], None).await?;
        if process["InvocationID"] != invocation {
            invocation = process["InvocationID"].clone();
            first_seen = Utc::now();
        }
        if let Ok(status) = server.node_status(&node.node_id).await {
            let sequence = if process["digest"] == identities["old"] {
                1
            } else {
                2
            };
            let version = version(sequence);
            let actual_digest = process["digest"]
                .as_str()
                .is_some_and(|value| identities["old"] == value || identities["new"] == value);
            if actual_digest
                && process["pid"].as_u64().unwrap_or(0) > 0
                && process["ActiveState"] == "active"
                && status.online
                && status.identity.agent_version == version
                && status.snapshot.as_ref().is_some_and(|snapshot| {
                    snapshot.collected_at > first_seen
                        && (Utc::now() - snapshot.collected_at).num_milliseconds() < 1500
                })
            {
                let collected_at = status
                    .snapshot
                    .context("observed metrics disappeared")?
                    .collected_at;
                let health = json!({"pid": process["pid"], "digest": process["digest"],
                    "sequence": sequence, "time": collected_at.timestamp_millis() as f64 / 1000.0});
                lab(&["health"], Some(&health.to_string())).await?;
            }
        }
        sleep(Duration::from_millis(150)).await;
    }
    Ok(())
}

async fn terminal(expected: &str) -> Result<Value> {
    timeout(Duration::from_secs(35), async {
        loop {
            let state = lab(&["state"], None).await?;
            if matches!(state["phase"].as_str(), Some("committed" | "rolled_back")) {
                ensure!(state["phase"] == expected, "unexpected terminal phase");
                return Ok(state);
            }
            sleep(Duration::from_millis(150)).await;
        }
    })
    .await
    .context("updater transaction did not complete")?
}

async fn credential_survives(server: &TestServer, node: &TestNode) -> Result<()> {
    let token = saved_token()?;
    ensure!(
        server.is_token_current(&node.node_id, &token).await,
        "disk credential rejected"
    );
    let registry = NodeRegistry::load(server.registry.path()).await?;
    let identity = server.node_status(&node.node_id).await?.identity;
    ensure!(
        registry.authorize(&identity, &token).await.is_ok(),
        "cold registry rejected disk token"
    );
    Ok(())
}

async fn interrupted_refresh(
    server: &TestServer,
    node: &TestNode,
    proxy: &proxy::Proxy,
    after_write: bool,
) -> Result<()> {
    let before = saved_token()?;
    proxy.gate.arm(if after_write { 2 } else { 1 });
    let interruption = async {
        timeout(Duration::from_secs(10), proxy.gate.reached.notified()).await?;
        ensure!(
            (saved_token()? != before) == after_write,
            "unexpected token durability window"
        );
        lab(&["signal", "SIGKILL"], None).await?;
        proxy.gate.release.notify_one();
        online(server, node, &version(1)).await
    };
    let (refresh, recovery) = tokio::join!(
        server.request_live_token_refresh(&node.node_id),
        interruption
    );
    recovery?;
    ensure!(
        refresh.is_err(),
        "dropped ACK unexpectedly committed the request"
    );
    lab(&["request"], None).await?;
    terminal("committed").await?;
    online(server, node, &version(2)).await?;
    credential_survives(server, node).await
}

async fn checkpoint_update(point: &str) -> Result<()> {
    lab(&["fault", point], None).await?;
    lab(&["request"], None).await?;
    timeout(Duration::from_secs(15), async {
        while !Path::new(CHECKPOINT).exists() {
            sleep(Duration::from_millis(100)).await;
        }
    })
    .await?;
    let state = lab(&["state"], None).await?;
    if point != "stopping" {
        let process = lab(&["probe"], None).await?;
        ensure!(
            process["pid"] == 0,
            "old process remained alive beyond switch barrier"
        );
    }
    ensure!(
        state["phase"]
            == if point == "stopping" {
                "stopping"
            } else {
                "trial"
            },
        "unexpected checkpoint phase"
    );
    lab(&["clear-fault"], None).await?;
    terminal(if point == "stopping" {
        "committed"
    } else {
        "rolled_back"
    })
    .await?;
    Ok(())
}

async fn unwritable_update(server: &TestServer, node: &TestNode) -> Result<()> {
    let original = saved_token()?;
    lab(&["writable", "no"], None).await?;
    let refresh = server.request_live_token_refresh(&node.node_id).await;
    lab(&["writable", "yes"], None).await?;
    ensure!(
        refresh.is_err(),
        "unwritable directory unexpectedly acknowledged token"
    );
    ensure!(
        saved_token()? == original,
        "failed write changed credentials"
    );
    lab(&["request"], None).await?;
    terminal("committed").await?;
    Ok(())
}

async fn stale_disk(server: &TestServer, node: &TestNode) -> Result<()> {
    let original = std::fs::read_to_string(CONFIG)?;
    let process = lab(&["probe"], None).await?;
    let mut stale = original.parse::<toml_edit::DocumentMut>()?;
    let mut random = [0u8; 32];
    getrandom::fill(&mut random).map_err(|_| anyhow::anyhow!("generate stale fixture token"))?;
    stale["agent"]["token"] = toml_edit::value(hex::encode(random));
    lab(&["config"], Some(&stale.to_string())).await?;
    let rejected = async {
        lab(&["request"], None).await?;
        let state = terminal("rolled_back").await?;
        ensure!(
            !state["rollback_consumed"].as_bool().unwrap_or(true),
            "preflight stopped live service"
        );
        ensure!(
            lab(&["probe"], None).await?["pid"] == process["pid"],
            "healthy RAM session was replaced"
        );
        online(server, node, &version(1)).await?;
        ensure!(
            !server
                .is_token_current(&node.node_id, &saved_token()?)
                .await,
            "stale disk token accepted"
        );
        Ok::<_, anyhow::Error>(())
    }
    .await;
    lab(&["config"], Some(&original)).await?;
    rejected
}

async fn rotate_update(server: &TestServer, node: &TestNode, rollback: bool) -> Result<()> {
    let before = saved_token()?;
    server.request_live_token_refresh(&node.node_id).await?;
    ensure!(saved_token()? != before, "rotation did not reach disk");
    lab(&["request"], None).await?;
    if rollback {
        online(server, node, &version(2)).await?;
        server.request_live_token_refresh(&node.node_id).await?;
        let latest = std::fs::read(CONFIG)?;
        lab(&["signal", "SIGSTOP"], None).await?;
        terminal("rolled_back").await?;
        ensure!(
            std::fs::read(CONFIG)? == latest,
            "binary rollback reverted latest config"
        );
    } else {
        terminal("committed").await?;
    }
    Ok(())
}

async fn released_upgrade(server: &TestServer, node: &TestNode, rollback: bool) -> Result<()> {
    lab(&["request"], None).await?;
    online(server, node, &version(2)).await?;
    let original = saved_token()?;
    server.request_live_token_refresh(&node.node_id).await?;
    ensure!(
        saved_token()? != original,
        "released candidate did not save token"
    );
    let latest = std::fs::read(CONFIG)?;
    if rollback {
        lab(&["signal", "SIGSTOP"], None).await?;
    }
    terminal(if rollback { "rolled_back" } else { "committed" }).await?;
    ensure!(
        std::fs::read(CONFIG)? == latest,
        "handoff reverted credentials"
    );
    Ok(())
}

async fn refresh_during_stop(
    server: &TestServer,
    node: &TestNode,
    proxy: &proxy::Proxy,
) -> Result<()> {
    proxy.gate.arm(2);
    let update = async {
        timeout(Duration::from_secs(10), proxy.gate.reached.notified()).await?;
        lab(&["request"], None).await?;
        let result = terminal("committed").await;
        proxy.gate.release.notify_one();
        result?;
        Ok::<_, anyhow::Error>(())
    };
    let (_, update) = tokio::join!(server.request_live_token_refresh(&node.node_id), update);
    update
}

async fn exercise(
    name: &str,
    server: &TestServer,
    node: &TestNode,
    proxy: &proxy::Proxy,
) -> Result<()> {
    online(server, node, &version(1)).await?;
    match name {
        "permissions" => {
            lab(&["permissions"], None).await?;
            return credential_survives(server, node).await;
        }
        "stale_disk" => stale_disk(server, node).await?,
        "lost_ack" | "killed_before_delivery" => {
            return interrupted_refresh(server, node, proxy, name == "lost_ack").await;
        }
        "stopping_crash" | "pointer_crash" | "trial_journal_crash" => {
            let point = match name {
                "stopping_crash" => "stopping",
                "pointer_crash" => "active_pointer",
                _ => "trial_journal",
            };
            checkpoint_update(point).await?;
        }
        "write_rejected" => {
            unwritable_update(server, node).await?;
        }
        "refresh_vs_stop" => {
            refresh_during_stop(server, node, proxy).await?;
        }
        "rotate_upgrade" | "rotate_rollback" => {
            rotate_update(server, node, name == "rotate_rollback").await?;
        }
        "upgrade_then_rotate" | "rotate_candidate_rollback" => {
            released_upgrade(server, node, name == "rotate_candidate_rollback").await?;
        }
        "legacy_refresh_rejected" => {
            let original = std::fs::read(CONFIG)?;
            let process = lab(&["probe"], None).await?;
            ensure!(
                server
                    .request_live_token_refresh(&node.node_id)
                    .await
                    .is_err()
            );
            ensure!(
                std::fs::read(CONFIG)? == original,
                "legacy refresh altered credentials"
            );
            ensure!(lab(&["probe"], None).await?["pid"] == process["pid"]);
            lab(&["request"], None).await?;
            terminal("committed").await?;
        }
        _ => bail!("unknown real Agent scenario"),
    }
    let state = lab(&["state"], None).await?;
    online(
        server,
        node,
        &version(if state["phase"] == "committed" { 2 } else { 1 }),
    )
    .await?;
    credential_survives(server, node).await
}

async fn scenario(name: &str) -> Result<Value> {
    let server = TestServer::start().await?;
    let node = server
        .issue_node(&format!("real-{name}"), "Isolated Real Agent")
        .await?;
    let proxy = proxy::Proxy::start(server.addr).await?;
    let config = format!(
        "[agent]\nnode_id = {:?}\nnode_label = {:?}\nserver = {:?}\ntoken = {:?}\nreport_interval_secs = 1\n",
        node.node_id, node.node_label, proxy.url, node.token
    );
    let setup = lab(&["provision"], Some(&config)).await;
    let result = if let Ok(ref identities) = setup {
        let stop = CancellationToken::new();
        let run = async {
            let result = timeout(
                Duration::from_secs(100),
                exercise(name, &server, &node, &proxy),
            )
            .await;
            stop.cancel();
            result.context("real Agent scenario exceeded deadline")?
        };
        let (run, observer) = tokio::join!(run, observe(&server, &node, identities, stop.clone()));
        run.and(observer)
    } else {
        setup.map(|_| ())
    };
    let cleanup = lab(&["halt"], None).await;
    proxy.close().await?;
    server.shutdown().await?;
    cleanup?;
    result.with_context(|| format!("isolated scenario {name} failed"))?;
    Ok(json!({"name": name, "passed": true}))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires root in explicitly marked disposable KVM guest; never run on compiler host"]
async fn real_agent_token_handoff_in_disposable_vm() -> Result<()> {
    lab(&["halt"], None).await?;
    let mut evidence = Vec::new();
    let current_cases = [
        "permissions",
        "stale_disk",
        "rotate_upgrade",
        "rotate_rollback",
        "write_rejected",
        "lost_ack",
        "killed_before_delivery",
        "refresh_vs_stop",
        "stopping_crash",
        "trial_journal_crash",
        "pointer_crash",
    ];
    let released_cases = [
        "permissions",
        "stale_disk",
        "legacy_refresh_rejected",
        "upgrade_then_rotate",
        "rotate_candidate_rollback",
        "stopping_crash",
        "trial_journal_crash",
        "pointer_crash",
    ];
    let cases = if version(1) == "3.0.3" {
        &released_cases[..]
    } else {
        &current_cases[..]
    };
    for name in cases {
        let result = scenario(name).await?;
        println!("{result}");
        evidence.push(result);
        std::fs::write(
            "/home/lab/real-agent-results.json",
            serde_json::to_vec_pretty(&evidence)?,
        )?;
    }
    Ok(())
}
