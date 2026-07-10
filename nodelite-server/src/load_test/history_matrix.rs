//! History-only SQLite concurrency and page-cache matrix benchmark.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use tokio::sync::Barrier;
use tokio::task::JoinSet;

use super::diagnostics::{ProcessMemorySnapshot, current_process_memory};
use super::probes::summarize_latencies;
use crate::history::HistoryStore;

mod process;

const MATRIX_NODE_COUNT: usize = 1_000;
const MATRIX_POINTS_PER_NODE: usize = 480;
const MATRIX_SAMPLE_INTERVAL: Duration = Duration::from_millis(10);

const MATRIX_CASES: [MatrixCase; 10] = [
    MatrixCase::baseline(),
    MatrixCase::configured("concurrency-2-cache-256", 2, 256),
    MatrixCase::configured("concurrency-2-cache-512", 2, 512),
    MatrixCase::configured("concurrency-2-cache-1024", 2, 1024),
    MatrixCase::configured("concurrency-4-cache-256", 4, 256),
    MatrixCase::configured("concurrency-4-cache-512", 4, 512),
    MatrixCase::configured("concurrency-4-cache-1024", 4, 1024),
    MatrixCase::configured("concurrency-8-cache-256", 8, 256),
    MatrixCase::configured("concurrency-8-cache-512", 8, 512),
    MatrixCase::configured("concurrency-8-cache-1024", 8, 1024),
];

#[derive(Clone, Copy, Debug)]
struct MatrixCase {
    label: &'static str,
    query_concurrency: usize,
    read_cache_kib: Option<u64>,
}

impl MatrixCase {
    const fn baseline() -> Self {
        Self {
            label: "legacy-baseline",
            query_concurrency: MATRIX_NODE_COUNT,
            read_cache_kib: None,
        }
    }

    const fn configured(
        label: &'static str,
        query_concurrency: usize,
        read_cache_kib: u64,
    ) -> Self {
        Self {
            label,
            query_concurrency,
            read_cache_kib: Some(read_cache_kib),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct MatrixCaseResult {
    label: String,
    query_concurrency: usize,
    read_cache_kib: Option<u64>,
    p50_ms: f64,
    p95_ms: f64,
    max_ms: f64,
    idle_memory: MatrixMemory,
    peak_memory: MatrixMemory,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
struct MatrixMemory {
    rss_bytes: u64,
    pss_bytes: Option<u64>,
    rss_anon_bytes: Option<u64>,
}

impl MatrixMemory {
    fn observe(&mut self, sample: ProcessMemorySnapshot) {
        self.rss_bytes = self.rss_bytes.max(sample.rss_bytes);
        self.pss_bytes = max_optional(self.pss_bytes, sample.pss_bytes);
        self.rss_anon_bytes = max_optional(self.rss_anon_bytes, sample.rss_anon_bytes);
    }
}

impl From<ProcessMemorySnapshot> for MatrixMemory {
    fn from(value: ProcessMemorySnapshot) -> Self {
        Self {
            rss_bytes: value.rss_bytes,
            pss_bytes: value.pss_bytes,
            rss_anon_bytes: value.rss_anon_bytes,
        }
    }
}

pub(super) async fn run_history_query_matrix() -> Result<()> {
    let temp_dir = matrix_temp_dir()?;
    tokio::fs::create_dir_all(&temp_dir)
        .await
        .with_context(|| format!("create matrix temp dir {}", temp_dir.display()))?;
    let db_path = temp_dir.join("history.sqlite3");
    let (start, end) = seed_matrix_database(&db_path).await?;
    warm_covering_index(&db_path).await?;

    println!(
        "HISTORY_QUERY_MATRIX starting nodes={} points_per_node={} cases={} independent_processes=true sample_interval_ms={}",
        MATRIX_NODE_COUNT,
        MATRIX_POINTS_PER_NODE,
        MATRIX_CASES.len(),
        MATRIX_SAMPLE_INTERVAL.as_millis(),
    );
    if !cfg!(target_os = "linux") {
        println!(
            "HISTORY_QUERY_MATRIX_MEMORY_LIMITATION platform={} rss_only=true pss_rssanon_require_linux=true",
            std::env::consts::OS,
        );
    }

    let mut baseline = None;
    for (case_index, case) in MATRIX_CASES.into_iter().enumerate() {
        let result =
            process::run_case_process(&temp_dir, &db_path, start, end, case_index, case).await?;
        let baseline_result = baseline.as_ref().unwrap_or(&result);
        print_matrix_result(&result, baseline_result);
        if baseline.is_none() {
            baseline = Some(result);
        }
    }

    remove_sqlite_artifacts(&db_path).await;
    let _ = tokio::fs::remove_dir(&temp_dir).await;
    Ok(())
}

pub(super) fn child_case_requested() -> bool {
    process::child_case_requested()
}

pub(super) async fn run_history_query_matrix_child() -> Result<()> {
    process::run_history_query_matrix_child().await
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
    label: String,
    query_concurrency: usize,
    read_cache_kib: Option<u64>,
) -> Result<MatrixCaseResult> {
    let store = match read_cache_kib {
        Some(read_cache_kib) => {
            HistoryStore::new(db_path.to_path_buf(), 5, query_concurrency, read_cache_kib)
        }
        None => {
            HistoryStore::new_with_default_read_cache(db_path.to_path_buf(), 5, query_concurrency)
        }
    };
    store.initialize().await;
    if !store.is_available() {
        bail!("history database unavailable for matrix case");
    }

    let result =
        measure_matrix_case(&store, start, end, label, query_concurrency, read_cache_kib).await;
    store.shutdown().await;
    result
}

async fn measure_matrix_case(
    store: &HistoryStore,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    label: String,
    query_concurrency: usize,
    read_cache_kib: Option<u64>,
) -> Result<MatrixCaseResult> {
    let idle_memory = current_process_memory()?;
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

    let mut peak_memory = MatrixMemory::from(idle_memory);
    peak_memory.observe(current_process_memory()?);
    start_barrier.wait().await;
    let mut sample_interval = tokio::time::interval(MATRIX_SAMPLE_INTERVAL);
    sample_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut latencies = Vec::with_capacity(MATRIX_NODE_COUNT);
    while !tasks.is_empty() {
        tokio::select! {
            _ = sample_interval.tick() => {
                peak_memory.observe(current_process_memory()?);
            }
            result = tasks.join_next() => {
                if let Some(result) = result {
                    latencies.push(result.context("join history matrix query task")??);
                }
            }
        }
    }
    peak_memory.observe(current_process_memory()?);

    let latency = summarize_latencies(&latencies)?;
    Ok(MatrixCaseResult {
        label,
        query_concurrency,
        read_cache_kib,
        p50_ms: latency.p50_ms,
        p95_ms: latency.p95_ms,
        max_ms: latency.max_ms,
        idle_memory: idle_memory.into(),
        peak_memory,
    })
}

fn print_matrix_result(result: &MatrixCaseResult, baseline: &MatrixCaseResult) {
    let rss_delta = memory_delta(result.peak_memory.rss_bytes, result.idle_memory.rss_bytes);
    let baseline_rss_delta = memory_delta(
        baseline.peak_memory.rss_bytes,
        baseline.idle_memory.rss_bytes,
    );
    let pss_delta =
        optional_memory_delta(result.peak_memory.pss_bytes, result.idle_memory.pss_bytes);
    let baseline_pss_delta = optional_memory_delta(
        baseline.peak_memory.pss_bytes,
        baseline.idle_memory.pss_bytes,
    );
    let rss_anon_delta = optional_memory_delta(
        result.peak_memory.rss_anon_bytes,
        result.idle_memory.rss_anon_bytes,
    );
    let baseline_rss_anon_delta = optional_memory_delta(
        baseline.peak_memory.rss_anon_bytes,
        baseline.idle_memory.rss_anon_bytes,
    );
    println!(
        "HISTORY_QUERY_MATRIX_RESULT case={} nodes={} points_per_node={} concurrency={} read_cache_kib={} p50_ms={:.2} p95_ms={:.2} max_ms={:.2} p95_delta_vs_baseline_ms={:+.2} idle_rss_bytes={} peak_rss_bytes={} peak_rss_delta_bytes={} peak_rss_delta_vs_baseline_bytes={} idle_pss_bytes={} peak_pss_bytes={} peak_pss_delta_bytes={} peak_pss_delta_vs_baseline_bytes={} idle_rss_anon_bytes={} peak_rss_anon_bytes={} peak_rss_anon_delta_bytes={} peak_rss_anon_delta_vs_baseline_bytes={}",
        result.label,
        MATRIX_NODE_COUNT,
        MATRIX_POINTS_PER_NODE,
        result.query_concurrency,
        read_cache_label(result.read_cache_kib),
        result.p50_ms,
        result.p95_ms,
        result.max_ms,
        result.p95_ms - baseline.p95_ms,
        result.idle_memory.rss_bytes,
        result.peak_memory.rss_bytes,
        rss_delta,
        signed_delta(rss_delta, baseline_rss_delta),
        optional_bytes(result.idle_memory.pss_bytes),
        optional_bytes(result.peak_memory.pss_bytes),
        optional_bytes(pss_delta),
        optional_signed_delta(pss_delta, baseline_pss_delta),
        optional_bytes(result.idle_memory.rss_anon_bytes),
        optional_bytes(result.peak_memory.rss_anon_bytes),
        optional_bytes(rss_anon_delta),
        optional_signed_delta(rss_anon_delta, baseline_rss_anon_delta),
    );
}

fn max_optional(left: Option<u64>, right: Option<u64>) -> Option<u64> {
    match (left, right) {
        (Some(left), Some(right)) => Some(left.max(right)),
        (Some(value), None) | (None, Some(value)) => Some(value),
        (None, None) => None,
    }
}

fn memory_delta(peak: u64, idle: u64) -> u64 {
    peak.saturating_sub(idle)
}

fn optional_memory_delta(peak: Option<u64>, idle: Option<u64>) -> Option<u64> {
    peak.zip(idle).map(|(peak, idle)| memory_delta(peak, idle))
}

fn signed_delta(value: u64, baseline: u64) -> i128 {
    i128::from(value) - i128::from(baseline)
}

fn optional_signed_delta(value: Option<u64>, baseline: Option<u64>) -> String {
    value.zip(baseline).map_or_else(
        || "unavailable".to_string(),
        |(value, baseline)| signed_delta(value, baseline).to_string(),
    )
}

fn optional_bytes(value: Option<u64>) -> String {
    value.map_or_else(|| "unavailable".to_string(), |value| value.to_string())
}

fn read_cache_label(value: Option<u64>) -> String {
    value.map_or_else(|| "sqlite_default".to_string(), |value| value.to_string())
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
