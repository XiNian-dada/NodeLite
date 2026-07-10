//! Independent child-process orchestration for history matrix cases.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};

use super::{MatrixCase, MatrixCaseResult, run_matrix_case};

const MATRIX_CHILD_TEST: &str = "load_test::load_test_history_query_matrix_case";
const ENV_DB_PATH: &str = "NODELITE_HISTORY_MATRIX_DB_PATH";
const ENV_START_TIMESTAMP: &str = "NODELITE_HISTORY_MATRIX_START_TIMESTAMP";
const ENV_END_TIMESTAMP: &str = "NODELITE_HISTORY_MATRIX_END_TIMESTAMP";
const ENV_CASE_LABEL: &str = "NODELITE_HISTORY_MATRIX_CASE_LABEL";
const ENV_QUERY_CONCURRENCY: &str = "NODELITE_HISTORY_MATRIX_QUERY_CONCURRENCY";
const ENV_READ_CACHE_KIB: &str = "NODELITE_HISTORY_MATRIX_READ_CACHE_KIB";
const ENV_RESULT_PATH: &str = "NODELITE_HISTORY_MATRIX_RESULT_PATH";

pub(super) fn child_case_requested() -> bool {
    std::env::var_os(ENV_DB_PATH).is_some()
}

pub(super) async fn run_history_query_matrix_child() -> Result<()> {
    let db_path = required_path_env(ENV_DB_PATH)?;
    let result_path = required_path_env(ENV_RESULT_PATH)?;
    let start = timestamp_env(ENV_START_TIMESTAMP)?;
    let end = timestamp_env(ENV_END_TIMESTAMP)?;
    let label = required_string_env(ENV_CASE_LABEL)?;
    let query_concurrency = parse_env::<usize>(ENV_QUERY_CONCURRENCY)?;
    let read_cache_kib = optional_u64_env(ENV_READ_CACHE_KIB)?;
    let result = run_matrix_case(
        &db_path,
        start,
        end,
        label,
        query_concurrency,
        read_cache_kib,
    )
    .await?;
    let encoded = serde_json::to_vec(&result).context("encode history matrix child result")?;
    tokio::fs::write(&result_path, encoded)
        .await
        .with_context(|| format!("write history matrix result {}", result_path.display()))?;
    Ok(())
}

pub(super) async fn run_case_process(
    temp_dir: &Path,
    db_path: &Path,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    case_index: usize,
    case: MatrixCase,
) -> Result<MatrixCaseResult> {
    let executable = std::env::current_exe().context("locate history matrix test executable")?;
    let result_path = temp_dir.join(format!("case-{case_index}.json"));
    let db_path = db_path.to_path_buf();
    let child_result_path = result_path.clone();
    let output = tokio::task::spawn_blocking(move || {
        Command::new(executable)
            .arg(MATRIX_CHILD_TEST)
            .args(["--exact", "--ignored", "--nocapture"])
            .env(ENV_DB_PATH, db_path)
            .env(ENV_START_TIMESTAMP, start.timestamp().to_string())
            .env(ENV_END_TIMESTAMP, end.timestamp().to_string())
            .env(ENV_CASE_LABEL, case.label)
            .env(ENV_QUERY_CONCURRENCY, case.query_concurrency.to_string())
            .env(
                ENV_READ_CACHE_KIB,
                case.read_cache_kib.map_or_else(
                    || "default".to_string(),
                    |read_cache_kib| read_cache_kib.to_string(),
                ),
            )
            .env(ENV_RESULT_PATH, child_result_path)
            .output()
            .context("run isolated history matrix case")
    })
    .await
    .context("join isolated history matrix process")??;
    if !output.status.success() {
        bail!(
            "history matrix case {} failed with {}\nstdout:\n{}\nstderr:\n{}",
            case.label,
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
    }

    let encoded = tokio::fs::read(&result_path)
        .await
        .with_context(|| format!("read history matrix result {}", result_path.display()))?;
    let result = serde_json::from_slice(&encoded).context("decode history matrix child result")?;
    let _ = tokio::fs::remove_file(&result_path).await;
    Ok(result)
}

fn required_path_env(name: &str) -> Result<PathBuf> {
    std::env::var_os(name)
        .map(PathBuf::from)
        .with_context(|| format!("missing {name}"))
}

fn required_string_env(name: &str) -> Result<String> {
    std::env::var(name).with_context(|| format!("missing {name}"))
}

fn parse_env<T>(name: &str) -> Result<T>
where
    T: std::str::FromStr,
    T::Err: std::error::Error + Send + Sync + 'static,
{
    required_string_env(name)?
        .parse::<T>()
        .with_context(|| format!("parse {name}"))
}

fn timestamp_env(name: &str) -> Result<DateTime<Utc>> {
    let timestamp = parse_env::<i64>(name)?;
    DateTime::from_timestamp(timestamp, 0).with_context(|| format!("invalid {name}"))
}

fn optional_u64_env(name: &str) -> Result<Option<u64>> {
    let value = required_string_env(name)?;
    if value == "default" {
        return Ok(None);
    }
    value
        .parse::<u64>()
        .map(Some)
        .with_context(|| format!("parse {name}"))
}
