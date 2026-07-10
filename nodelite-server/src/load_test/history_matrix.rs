//! History-only SQLite concurrency and page-cache matrix benchmark.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};
use rusqlite::{Connection, params};
use tokio::sync::Barrier;
use tokio::task::JoinSet;

use super::diagnostics::{ProcessMemorySnapshot, current_process_memory};
use super::probes::summarize_latencies;
use crate::history::HistoryStore;

const MATRIX_NODE_COUNT: usize = 1_000;
const MATRIX_POINTS_PER_NODE: usize = 480;
const MATRIX_QUERY_CONCURRENCY: [usize; 3] = [2, 4, 8];
const MATRIX_READ_CACHE_KIB: [u64; 3] = [256, 512, 1024];

pub(super) async fn run_history_query_matrix() -> Result<()> {
    let temp_dir = matrix_temp_dir()?;
    tokio::fs::create_dir_all(&temp_dir)
        .await
        .with_context(|| format!("create matrix temp dir {}", temp_dir.display()))?;
    let db_path = temp_dir.join("history.sqlite3");
    let (start, end) = seed_matrix_database(&db_path).await?;
    warm_covering_index(&db_path).await?;

    println!(
        "HISTORY_QUERY_MATRIX starting nodes={} points_per_node={} combinations={}",
        MATRIX_NODE_COUNT,
        MATRIX_POINTS_PER_NODE,
        MATRIX_QUERY_CONCURRENCY.len() * MATRIX_READ_CACHE_KIB.len(),
    );
    if !cfg!(target_os = "linux") {
        println!(
            "HISTORY_QUERY_MATRIX_MEMORY_LIMITATION platform={} rss_only=true pss_rssanon_require_linux=true",
            std::env::consts::OS,
        );
    }

    for query_concurrency in MATRIX_QUERY_CONCURRENCY {
        for read_cache_kib in MATRIX_READ_CACHE_KIB {
            run_matrix_case(&db_path, start, end, query_concurrency, read_cache_kib).await?;
        }
    }

    remove_sqlite_artifacts(&db_path).await;
    let _ = tokio::fs::remove_dir(&temp_dir).await;
    Ok(())
}

async fn warm_covering_index(db_path: &Path) -> Result<()> {
    let db_path = db_path.to_path_buf();
    tokio::task::spawn_blocking(move || {
        let connection = Connection::open(&db_path)
            .with_context(|| format!("open history matrix database {}", db_path.display()))?;
        let _: Option<i64> = connection
            .query_row(
                "SELECT SUM(recorded_at) FROM history_points INDEXED BY idx_history_points_covering_metrics",
                [],
                |row| row.get(0),
            )
            .context("warm history covering index")?;
        anyhow::Ok(())
    })
    .await
    .context("join history matrix warmup task")?
}

async fn seed_matrix_database(db_path: &Path) -> Result<(DateTime<Utc>, DateTime<Utc>)> {
    let bootstrap = HistoryStore::new(
        db_path.to_path_buf(),
        5,
        nodelite_proto::DEFAULT_HISTORY_QUERY_CONCURRENCY,
        nodelite_proto::DEFAULT_HISTORY_READ_CACHE_KIB,
    );
    bootstrap.initialize().await;
    if !bootstrap.is_available() {
        bail!("history database failed to initialize for matrix benchmark");
    }
    bootstrap.shutdown().await;

    let db_path = db_path.to_path_buf();
    tokio::task::spawn_blocking(move || seed_rows(&db_path))
        .await
        .context("join history matrix seed task")?
}

fn seed_rows(db_path: &Path) -> Result<(DateTime<Utc>, DateTime<Utc>)> {
    let mut connection = Connection::open(db_path)
        .with_context(|| format!("open history matrix database {}", db_path.display()))?;
    let transaction = connection
        .transaction()
        .context("start history matrix seed transaction")?;
    let end = Utc::now();
    let start =
        end - chrono::Duration::seconds((MATRIX_POINTS_PER_NODE.saturating_sub(1) * 30) as i64);
    {
        let mut statement = transaction
            .prepare(
                r#"
                INSERT INTO history_points (
                    node_id, recorded_at, cpu_usage_percent,
                    load_one, load_five, load_fifteen,
                    memory_used_percent, rx_bytes_per_sec, tx_bytes_per_sec,
                    latency_ms, packet_loss_percent, disk_used_percent
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
                "#,
            )
            .context("prepare history matrix seed insert")?;
        for node_index in 0..MATRIX_NODE_COUNT {
            let node_id = format!("matrix-node-{node_index:04}");
            for point_index in 0..MATRIX_POINTS_PER_NODE {
                let recorded_at = start + chrono::Duration::seconds((point_index * 30) as i64);
                statement.execute(params![
                    node_id,
                    recorded_at.timestamp(),
                    (point_index % 100) as f64,
                    point_index as f64 / 100.0,
                    point_index as f64 / 200.0,
                    point_index as f64 / 300.0,
                    50.0 + (point_index % 20) as f64,
                    point_index as f64 * 1024.0,
                    point_index as f64 * 512.0,
                    (point_index % 50) as u64,
                    (point_index % 10) as f64 / 10.0,
                    60.0,
                ])?;
            }
        }
    }
    transaction
        .commit()
        .context("commit history matrix seed transaction")?;
    Ok((start, end))
}

async fn run_matrix_case(
    db_path: &Path,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    query_concurrency: usize,
    read_cache_kib: u64,
) -> Result<()> {
    let store = HistoryStore::new(db_path.to_path_buf(), 5, query_concurrency, read_cache_kib);
    store.initialize().await;
    if !store.is_available() {
        bail!("history database unavailable for matrix case");
    }

    let start_barrier = Arc::new(Barrier::new(MATRIX_NODE_COUNT + 1));
    let mut tasks = JoinSet::new();
    for node_index in 0..MATRIX_NODE_COUNT {
        let store = store.clone();
        let start_barrier = Arc::clone(&start_barrier);
        tasks.spawn(async move {
            let node_id = format!("matrix-node-{node_index:04}");
            start_barrier.wait().await;
            let started = Instant::now();
            let points = store
                .query_history_range(&node_id, start, end, MATRIX_POINTS_PER_NODE)
                .await
                .with_context(|| format!("query matrix history for {node_id}"))?;
            if points.len() != MATRIX_POINTS_PER_NODE {
                bail!(
                    "matrix query returned {} / {} points for {node_id}",
                    points.len(),
                    MATRIX_POINTS_PER_NODE,
                );
            }
            anyhow::Ok(started.elapsed())
        });
    }
    start_barrier.wait().await;

    let mut latencies = Vec::with_capacity(MATRIX_NODE_COUNT);
    while let Some(result) = tasks.join_next().await {
        latencies.push(result.context("join history matrix query task")??);
    }
    let latency = summarize_latencies(&latencies)?;
    let memory = current_process_memory()?;
    print_matrix_result(query_concurrency, read_cache_kib, latency, memory);
    store.shutdown().await;
    Ok(())
}

fn print_matrix_result(
    query_concurrency: usize,
    read_cache_kib: u64,
    latency: super::LatencySummary,
    memory: ProcessMemorySnapshot,
) {
    println!(
        "HISTORY_QUERY_MATRIX_RESULT nodes={} points_per_node={} concurrency={} read_cache_kib={} p50_ms={:.2} p95_ms={:.2} max_ms={:.2} rss_bytes={} pss_bytes={} rss_anon_bytes={}",
        MATRIX_NODE_COUNT,
        MATRIX_POINTS_PER_NODE,
        query_concurrency,
        read_cache_kib,
        latency.p50_ms,
        latency.p95_ms,
        latency.max_ms,
        memory.rss_bytes,
        optional_bytes(memory.pss_bytes),
        optional_bytes(memory.rss_anon_bytes),
    );
}

fn optional_bytes(value: Option<u64>) -> String {
    value.map_or_else(|| "unavailable".to_string(), |value| value.to_string())
}

fn matrix_temp_dir() -> Result<PathBuf> {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("clock should move forward")?
        .as_nanos();
    Ok(std::env::temp_dir().join(format!("nodelite-history-matrix-{unique}")))
}

async fn remove_sqlite_artifacts(db_path: &Path) {
    for suffix in ["", "-wal", "-shm"] {
        let path = PathBuf::from(format!("{}{suffix}", db_path.display()));
        let _ = tokio::fs::remove_file(path).await;
    }
}
