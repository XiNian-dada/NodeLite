//! Correlate cold/warm WebSocket cycles with real verifier counters and registry revisions.

use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, ensure};
use nodelite_server::bench_support::{NodeRegistry, token_verify_metrics};
use tokio::sync::{Barrier, mpsc};

use super::fake_agent::{run_fake_agent_session, wait_for_all_offline, wait_for_final_snapshots};
use super::probes::{probe_nodes_latencies, probe_overview_latencies, summarize_latencies};
use super::scenarios::wait_for_ready_nodes;
use super::server::TestServer;
use super::{
    AgentCredential, AgentWorkload, LOAD_TEST_STORM_CYCLES, LOAD_TEST_STORM_METRIC_DELAY_MS,
    LOAD_TEST_STORM_METRICS_PER_CYCLE, LOAD_TEST_STORM_READ_PROBES, LOAD_TEST_TIMEOUT_SECS,
    StormScenarioResult,
};

pub(super) async fn run(node_count: usize) -> Result<StormScenarioResult> {
    let (server, credentials) = TestServer::start(node_count).await?;
    let registry = server.registry();
    let before = token_verify_metrics(&registry);
    let revision = registry.registry_revision();
    let mut connect = Vec::new();
    let mut recover = Vec::new();
    let mut disconnect = Vec::new();
    let mut overview = Vec::new();
    let mut nodes = Vec::new();
    for cycle in 0..LOAD_TEST_STORM_CYCLES {
        let result = run_cycle(&server, &credentials, cycle, &registry).await?;
        connect.push(result.connect);
        recover.push(result.recover);
        disconnect.push(result.disconnect);
        overview.extend(result.overview);
        nodes.extend(result.nodes);
    }
    let after = token_verify_metrics(&registry);
    let hits = after.token_cache_hits_total - before.token_cache_hits_total;
    let misses = after.token_cache_misses_total - before.token_cache_misses_total;
    let evictions = after.token_cache_evictions_total - before.token_cache_evictions_total;
    let hit_percent = 100.0 * hits as f64 / (hits + misses).max(1) as f64;
    println!(
        "TOKEN_CACHE_RESULT nodes={node_count} cycles={LOAD_TEST_STORM_CYCLES} hits={hits} misses={misses} evictions={evictions} hit_percent={hit_percent:.2} revision_start={revision} revision_end={}",
        registry.registry_revision()
    );
    ensure!(
        registry.registry_revision() == revision,
        "pure reconnect changed registry revision"
    );
    ensure!(evictions == 0, "fleet fits cache but evictions occurred");
    ensure!(
        hit_percent > 60.0,
        "reconnect cache hit rate is below acceptance target"
    );
    server.shutdown().await?;
    Ok(StormScenarioResult {
        nodes: node_count,
        cycles: LOAD_TEST_STORM_CYCLES,
        sessions_total: node_count * LOAD_TEST_STORM_CYCLES,
        connect: summarize_latencies(&connect)?,
        recover: summarize_latencies(&recover)?,
        disconnect: summarize_latencies(&disconnect)?,
        overview: summarize_latencies(&overview)?,
        nodes_api: summarize_latencies(&nodes)?,
    })
}

struct CycleResult {
    connect: Duration,
    recover: Duration,
    disconnect: Duration,
    overview: Vec<Duration>,
    nodes: Vec<Duration>,
}

struct ConnectedCycle {
    connect: Duration,
    workload: AgentWorkload,
    barrier: Arc<Barrier>,
    handles: Vec<tokio::task::JoinHandle<Result<()>>>,
}

async fn connect_cycle(
    server: &TestServer,
    credentials: &[AgentCredential],
    cycle: usize,
    registry: &NodeRegistry,
) -> Result<ConnectedCycle> {
    let node_count = credentials.len();
    let before = token_verify_metrics(registry);
    let revision = registry.registry_revision();
    println!(
        "TOKEN_CACHE_CYCLE_BEGIN nodes={node_count} cycle={}",
        cycle + 1
    );
    let (ready_tx, mut ready_rx) = mpsc::unbounded_channel();
    let barrier = Arc::new(Barrier::new(node_count + 1));
    let workload = AgentWorkload {
        uptime_start: cycle as u64 * 1_000 + 1,
        metrics_per_node: LOAD_TEST_STORM_METRICS_PER_CYCLE,
        inter_message_delay: Duration::from_millis(LOAD_TEST_STORM_METRIC_DELAY_MS),
        hold_after_send: Duration::from_millis(250),
        disk_entries: 1,
    };
    let started = Instant::now();
    let handles: Vec<_> = credentials
        .iter()
        .cloned()
        .map(|credential| {
            tokio::spawn(run_fake_agent_session(
                server.addr,
                credential,
                workload,
                ready_tx.clone(),
                barrier.clone(),
            ))
        })
        .collect();
    drop(ready_tx);
    wait_for_ready_nodes(
        &mut ready_rx,
        node_count,
        &format!("storm cycle {}", cycle + 1),
    )
    .await?;
    let connect = started.elapsed();
    let after = token_verify_metrics(registry);
    println!(
        "TOKEN_CACHE_CYCLE_RESULT nodes={node_count} cycle={} hits={} misses={} evictions={} revision_start={revision} revision_end={} connect_ms={:.2}",
        cycle + 1,
        after.token_cache_hits_total - before.token_cache_hits_total,
        after.token_cache_misses_total - before.token_cache_misses_total,
        after.token_cache_evictions_total - before.token_cache_evictions_total,
        registry.registry_revision(),
        connect.as_secs_f64() * 1000.0
    );
    Ok(ConnectedCycle {
        connect,
        workload,
        barrier,
        handles,
    })
}

async fn run_cycle(
    server: &TestServer,
    credentials: &[AgentCredential],
    cycle: usize,
    registry: &NodeRegistry,
) -> Result<CycleResult> {
    let node_count = credentials.len();
    let ConnectedCycle {
        connect,
        workload,
        barrier,
        handles,
    } = connect_cycle(server, credentials, cycle, registry).await?;
    let overview = tokio::spawn(probe_overview_latencies(
        server.addr,
        LOAD_TEST_STORM_READ_PROBES,
    ));
    let nodes = tokio::spawn(probe_nodes_latencies(
        server.addr,
        LOAD_TEST_STORM_READ_PROBES,
        node_count,
    ));
    let started = Instant::now();
    barrier.wait().await;
    wait_for_final_snapshots(
        server.shared.clone(),
        credentials,
        workload.uptime_start + workload.metrics_per_node - 1,
        Duration::from_secs(LOAD_TEST_TIMEOUT_SECS),
        false,
    )
    .await?;
    let recover = started.elapsed();
    for handle in handles {
        handle.await.context("join storm Agent")??;
    }
    let started = Instant::now();
    wait_for_all_offline(
        server.shared.clone(),
        credentials,
        Duration::from_secs(LOAD_TEST_TIMEOUT_SECS),
    )
    .await?;
    Ok(CycleResult {
        connect,
        recover,
        disconnect: started.elapsed(),
        overview: overview.await.context("join storm overview probe")??,
        nodes: nodes.await.context("join storm nodes probe")??,
    })
}
