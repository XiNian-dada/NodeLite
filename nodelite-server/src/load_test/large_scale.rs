//! 大规模手动压测场景,覆盖 500/1000 节点、多 dashboard reader 与大 payload。

use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use tokio::sync::{Barrier, mpsc, watch};
use tokio::time::{sleep, timeout};

use super::diagnostics::{ResourceSnapshot, ViewCacheCounters};
use super::fake_agent::{
    SeededHistoryRange, run_fake_agent, seed_history_points, wait_for_final_snapshots,
    wait_for_seeded_history_points,
};
use super::probes::{
    BodySizeSummary, HttpProbeSample, probe_dashboard_refreshes, probe_metrics, probe_node_history,
    probe_nodes, probe_overview, summarize_probe_samples,
};
use super::server::TestServer;
use super::{AgentCredential, AgentWorkload};

const LARGE_TEST_TIMEOUT_SECS: u64 = 120;
const LARGE_FLEET_SCENARIOS: [usize; 2] = [500, 1000];
const LARGE_FLEET_METRICS_PER_NODE: u64 = 6;
const LARGE_FLEET_READ_SAMPLES: usize = 12;
const DASHBOARD_NODE_COUNT: usize = 1000;
const DASHBOARD_READERS: usize = 20;
const DASHBOARD_REFRESHES_PER_READER: usize = 4;
const DASHBOARD_METRICS_PER_NODE: u64 = 8;
const HISTORY_NODE_COUNT: usize = 1000;
const HISTORY_READERS: usize = 20;
const HISTORY_POINTS_PER_NODE: usize = 240;
const HISTORY_METRICS_PER_NODE: u64 = 4;
const PAYLOAD_NODE_COUNT: usize = 500;
const PAYLOAD_DISK_ENTRIES: usize = 64;
const PAYLOAD_METRICS_PER_NODE: u64 = 4;
const PROMETHEUS_SCRAPE_SAMPLES: usize = 8;

#[derive(Debug)]
struct WorkloadRun {
    connect_ms: f64,
    settle_ms: f64,
    metrics_total: usize,
    metrics_per_sec: f64,
}

pub(super) async fn run_large_fleet_load_test() -> Result<()> {
    println!(
        "LARGE_FLEET_TEST starting scenarios={:?} metrics_per_node={} read_samples={}",
        LARGE_FLEET_SCENARIOS, LARGE_FLEET_METRICS_PER_NODE, LARGE_FLEET_READ_SAMPLES,
    );
    for node_count in LARGE_FLEET_SCENARIOS {
        let (server, credentials) = TestServer::start(node_count).await?;
        let run = drive_agent_workload(
            &server,
            &credentials,
            AgentWorkload {
                uptime_start: 1,
                metrics_per_node: LARGE_FLEET_METRICS_PER_NODE,
                inter_message_delay: Duration::from_millis(5),
                hold_after_send: Duration::ZERO,
                disk_entries: 1,
            },
        )
        .await?;
        let overview_task = tokio::spawn(probe_overview(server.addr, LARGE_FLEET_READ_SAMPLES));
        let nodes_task = tokio::spawn(probe_nodes(
            server.addr,
            LARGE_FLEET_READ_SAMPLES,
            node_count,
        ));
        let metrics_task = tokio::spawn(probe_metrics(server.addr, PROMETHEUS_SCRAPE_SAMPLES));
        let overview = summarize_probe_samples(&join_probe_task(overview_task, "overview").await?)?;
        let nodes = summarize_probe_samples(&join_probe_task(nodes_task, "nodes").await?)?;
        let metrics = summarize_probe_samples(&join_probe_task(metrics_task, "metrics").await?)?;
        let resources = ResourceSnapshot::capture(&server).await?;
        println!(
            "LARGE_FLEET_RESULT nodes={} connect_ms={:.1} settle_ms={:.1} metrics_total={} metrics_per_sec={:.1} overview_p95_ms={:.2} nodes_p95_ms={:.2} metrics_p95_ms={:.2} overview_body_bytes={} nodes_body_bytes={} metrics_body_bytes={} rss_bytes={} history_queue_depth={} history_dropped_writes={} db_bytes={} wal_bytes={} shm_bytes={} {}",
            node_count,
            run.connect_ms,
            run.settle_ms,
            run.metrics_total,
            run.metrics_per_sec,
            overview.latency.p95_ms,
            nodes.latency.p95_ms,
            metrics.latency.p95_ms,
            body_bytes_triplet(overview.body_bytes),
            body_bytes_triplet(nodes.body_bytes),
            body_bytes_triplet(metrics.body_bytes),
            resources.rss_bytes,
            resources.history_queue_depth,
            resources.history_dropped_writes,
            resources.history_artifacts.db,
            resources.history_artifacts.wal,
            resources.history_artifacts.shm,
            view_cache_summary(resources.view_cache),
        );
        server.shutdown().await?;
    }
    Ok(())
}

pub(super) async fn run_dashboard_fanout_load_test() -> Result<()> {
    let (server, credentials) = TestServer::start(DASHBOARD_NODE_COUNT).await?;
    println!(
        "DASHBOARD_FANOUT_TEST starting nodes={} dashboard_readers={} refreshes_per_reader={} prometheus_scrapes={}",
        DASHBOARD_NODE_COUNT,
        DASHBOARD_READERS,
        DASHBOARD_REFRESHES_PER_READER,
        PROMETHEUS_SCRAPE_SAMPLES,
    );
    let run = drive_agent_workload(
        &server,
        &credentials,
        AgentWorkload {
            uptime_start: 1,
            metrics_per_node: DASHBOARD_METRICS_PER_NODE,
            inter_message_delay: Duration::from_millis(10),
            hold_after_send: Duration::ZERO,
            disk_entries: 1,
        },
    )
    .await?;
    let dashboard_tasks = (0..DASHBOARD_READERS)
        .map(|_| {
            tokio::spawn(probe_dashboard_refreshes(
                server.addr,
                DASHBOARD_REFRESHES_PER_READER,
                DASHBOARD_NODE_COUNT,
            ))
        })
        .collect::<Vec<_>>();
    let metrics_task = tokio::spawn(probe_metrics(server.addr, PROMETHEUS_SCRAPE_SAMPLES));
    let mut overview_samples =
        Vec::with_capacity(DASHBOARD_READERS * DASHBOARD_REFRESHES_PER_READER);
    let mut nodes_samples = Vec::with_capacity(DASHBOARD_READERS * DASHBOARD_REFRESHES_PER_READER);
    for task in dashboard_tasks {
        let samples = task
            .await
            .map_err(|error| anyhow!("join dashboard reader task: {error}"))??;
        overview_samples.extend(samples.overview);
        nodes_samples.extend(samples.nodes);
    }
    let overview = summarize_probe_samples(&overview_samples)?;
    let nodes = summarize_probe_samples(&nodes_samples)?;
    let metrics = summarize_probe_samples(&join_probe_task(metrics_task, "metrics").await?)?;
    let resources = ResourceSnapshot::capture(&server).await?;
    println!(
        "DASHBOARD_FANOUT_RESULT nodes={} dashboard_readers={} refreshes_total={} connect_ms={:.1} settle_ms={:.1} metrics_total={} metrics_per_sec={:.1} overview_p95_ms={:.2} nodes_p95_ms={:.2} metrics_p95_ms={:.2} overview_body_bytes={} nodes_body_bytes={} metrics_body_bytes={} rss_bytes={} history_queue_depth={} history_dropped_writes={} db_bytes={} wal_bytes={} shm_bytes={} {}",
        DASHBOARD_NODE_COUNT,
        DASHBOARD_READERS,
        DASHBOARD_READERS * DASHBOARD_REFRESHES_PER_READER,
        run.connect_ms,
        run.settle_ms,
        run.metrics_total,
        run.metrics_per_sec,
        overview.latency.p95_ms,
        nodes.latency.p95_ms,
        metrics.latency.p95_ms,
        body_bytes_triplet(overview.body_bytes),
        body_bytes_triplet(nodes.body_bytes),
        body_bytes_triplet(metrics.body_bytes),
        resources.rss_bytes,
        resources.history_queue_depth,
        resources.history_dropped_writes,
        resources.history_artifacts.db,
        resources.history_artifacts.wal,
        resources.history_artifacts.shm,
        view_cache_summary(resources.view_cache),
    );
    server.shutdown().await?;
    Ok(())
}

pub(super) async fn run_history_pressure_load_test() -> Result<()> {
    let (server, credentials) = TestServer::start(HISTORY_NODE_COUNT).await?;
    println!(
        "HISTORY_PRESSURE_TEST starting nodes={} history_readers={} history_points_per_node={}",
        HISTORY_NODE_COUNT, HISTORY_READERS, HISTORY_POINTS_PER_NODE,
    );
    let ranges = seed_history_for_readers(&server, &credentials).await?;
    let history_tasks = ranges
        .into_iter()
        .map(|(credential, range)| {
            tokio::spawn(probe_node_history(
                server.addr,
                credential.node_id,
                4,
                HISTORY_POINTS_PER_NODE.saturating_sub(8),
                range.start_at.timestamp(),
                range.end_at.timestamp(),
            ))
        })
        .collect::<Vec<_>>();
    let run = drive_agent_workload(
        &server,
        &credentials,
        AgentWorkload {
            uptime_start: 1,
            metrics_per_node: HISTORY_METRICS_PER_NODE,
            inter_message_delay: Duration::from_millis(10),
            hold_after_send: Duration::ZERO,
            disk_entries: 1,
        },
    )
    .await?;
    let mut history_samples = Vec::with_capacity(HISTORY_READERS * 4);
    for task in history_tasks {
        history_samples.extend(join_probe_task(task, "history").await?);
    }
    let history = summarize_probe_samples(&history_samples)?;
    let resources = ResourceSnapshot::capture(&server).await?;
    println!(
        "HISTORY_PRESSURE_RESULT nodes={} history_readers={} history_points_per_node={} connect_ms={:.1} settle_ms={:.1} metrics_total={} metrics_per_sec={:.1} history_p95_ms={:.2} history_body_bytes={} rss_bytes={} history_queue_depth={} history_dropped_writes={} db_bytes={} wal_bytes={} shm_bytes={} {}",
        HISTORY_NODE_COUNT,
        HISTORY_READERS,
        HISTORY_POINTS_PER_NODE,
        run.connect_ms,
        run.settle_ms,
        run.metrics_total,
        run.metrics_per_sec,
        history.latency.p95_ms,
        body_bytes_triplet(history.body_bytes),
        resources.rss_bytes,
        resources.history_queue_depth,
        resources.history_dropped_writes,
        resources.history_artifacts.db,
        resources.history_artifacts.wal,
        resources.history_artifacts.shm,
        view_cache_summary(resources.view_cache),
    );
    server.shutdown().await?;
    Ok(())
}

pub(super) async fn run_payload_size_load_test() -> Result<()> {
    let (server, credentials) = TestServer::start(PAYLOAD_NODE_COUNT).await?;
    println!(
        "PAYLOAD_SIZE_TEST starting nodes={} disk_entries_per_node={}",
        PAYLOAD_NODE_COUNT, PAYLOAD_DISK_ENTRIES,
    );
    let run = drive_agent_workload(
        &server,
        &credentials,
        AgentWorkload {
            uptime_start: 1,
            metrics_per_node: PAYLOAD_METRICS_PER_NODE,
            inter_message_delay: Duration::from_millis(10),
            hold_after_send: Duration::ZERO,
            disk_entries: PAYLOAD_DISK_ENTRIES,
        },
    )
    .await?;
    let nodes_task = tokio::spawn(probe_nodes(server.addr, 8, PAYLOAD_NODE_COUNT));
    let metrics_task = tokio::spawn(probe_metrics(server.addr, PROMETHEUS_SCRAPE_SAMPLES));
    let nodes = summarize_probe_samples(&join_probe_task(nodes_task, "nodes").await?)?;
    let metrics = summarize_probe_samples(&join_probe_task(metrics_task, "metrics").await?)?;
    let resources = ResourceSnapshot::capture(&server).await?;
    println!(
        "PAYLOAD_SIZE_RESULT nodes={} disk_entries_per_node={} connect_ms={:.1} settle_ms={:.1} metrics_total={} metrics_per_sec={:.1} nodes_p95_ms={:.2} metrics_p95_ms={:.2} nodes_body_bytes={} metrics_body_bytes={} rss_bytes={} history_queue_depth={} history_dropped_writes={} db_bytes={} wal_bytes={} shm_bytes={} {}",
        PAYLOAD_NODE_COUNT,
        PAYLOAD_DISK_ENTRIES,
        run.connect_ms,
        run.settle_ms,
        run.metrics_total,
        run.metrics_per_sec,
        nodes.latency.p95_ms,
        metrics.latency.p95_ms,
        body_bytes_triplet(nodes.body_bytes),
        body_bytes_triplet(metrics.body_bytes),
        resources.rss_bytes,
        resources.history_queue_depth,
        resources.history_dropped_writes,
        resources.history_artifacts.db,
        resources.history_artifacts.wal,
        resources.history_artifacts.shm,
        view_cache_summary(resources.view_cache),
    );
    server.shutdown().await?;
    Ok(())
}

fn body_bytes_triplet(summary: BodySizeSummary) -> String {
    format!("{}/{}/{}", summary.p50, summary.p95, summary.max)
}

fn view_cache_summary(counters: ViewCacheCounters) -> String {
    format!(
        "overview_hits={} overview_misses={} nodes_hits={} nodes_misses={} metrics_hits={} metrics_misses={}",
        counters.overview_hits,
        counters.overview_misses,
        counters.nodes_hits,
        counters.nodes_misses,
        counters.metrics_hits,
        counters.metrics_misses,
    )
}

async fn drive_agent_workload(
    server: &TestServer,
    credentials: &[AgentCredential],
    workload: AgentWorkload,
) -> Result<WorkloadRun> {
    let (ready_tx, mut ready_rx) = mpsc::unbounded_channel::<String>();
    let (stop_tx, stop_rx) = watch::channel(false);
    let burst_barrier = Arc::new(Barrier::new(credentials.len() + 1));
    let mut handles = Vec::with_capacity(credentials.len());
    let expected_final_uptime = workload.uptime_start + workload.metrics_per_node - 1;
    let connect_started = Instant::now();

    for credential in credentials.iter().cloned() {
        handles.push(tokio::spawn(run_fake_agent(
            server.addr,
            credential,
            workload,
            ready_tx.clone(),
            Arc::clone(&burst_barrier),
            stop_rx.clone(),
        )));
    }
    drop(ready_tx);

    wait_for_ready_nodes(&mut ready_rx, credentials.len(), "large-scale fake agents").await?;
    let connect_elapsed = connect_started.elapsed();

    let settle_started = Instant::now();
    burst_barrier.wait().await;
    wait_for_final_snapshots(
        server.shared.clone(),
        credentials,
        expected_final_uptime,
        Duration::from_secs(LARGE_TEST_TIMEOUT_SECS),
        true,
    )
    .await?;
    let settle_elapsed = settle_started.elapsed();

    let _ = stop_tx.send(true);
    for handle in handles {
        handle
            .await
            .map_err(|error| anyhow!("join large-scale fake agent task: {error}"))??;
    }

    let metrics_total = credentials.len() * workload.metrics_per_node as usize;
    let settle_secs = settle_elapsed.as_secs_f64().max(0.001);
    Ok(WorkloadRun {
        connect_ms: connect_elapsed.as_secs_f64() * 1000.0,
        settle_ms: settle_elapsed.as_secs_f64() * 1000.0,
        metrics_total,
        metrics_per_sec: metrics_total as f64 / settle_secs,
    })
}

async fn seed_history_for_readers(
    server: &TestServer,
    credentials: &[AgentCredential],
) -> Result<Vec<(AgentCredential, SeededHistoryRange)>> {
    let mut ranges = Vec::with_capacity(HISTORY_READERS);
    for credential in credentials.iter().take(HISTORY_READERS) {
        let range =
            seed_history_points(server.history.clone(), credential, HISTORY_POINTS_PER_NODE)
                .await?;
        wait_for_seeded_history_points(
            server.history.clone(),
            &credential.node_id,
            range,
            HISTORY_POINTS_PER_NODE,
        )
        .await?;
        ranges.push((credential.clone(), range));
    }
    Ok(ranges)
}

async fn join_probe_task(
    task: tokio::task::JoinHandle<Result<Vec<HttpProbeSample>>>,
    label: &str,
) -> Result<Vec<HttpProbeSample>> {
    task.await
        .map_err(|error| anyhow!("join {label} probe task: {error}"))?
}

async fn wait_for_ready_nodes(
    ready_rx: &mut mpsc::UnboundedReceiver<String>,
    node_count: usize,
    label: &str,
) -> Result<()> {
    let mut ready_nodes = std::collections::HashSet::with_capacity(node_count);
    while ready_nodes.len() < node_count {
        let next = timeout(
            Duration::from_secs(LARGE_TEST_TIMEOUT_SECS),
            ready_rx.recv(),
        )
        .await
        .with_context(|| format!("timed out waiting for {label} to authenticate"))?;
        let Some(node_id) = next else {
            bail!(
                "{label} ready channel closed early after {} / {} nodes",
                ready_nodes.len(),
                node_count
            );
        };
        ready_nodes.insert(node_id);
        if ready_nodes.len() % 100 == 0 {
            sleep(Duration::from_millis(1)).await;
        }
    }
    Ok(())
}
