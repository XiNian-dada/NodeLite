use std::net::SocketAddr;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::sleep;

use super::{LOAD_TEST_BASIC_AUTH, LatencySummary};
use nodelite_proto::{NodeListItem, NodeStatus};

#[derive(Debug, Clone, Copy)]
pub(super) struct HttpProbeSample {
    pub(super) latency: Duration,
    pub(super) body_bytes: usize,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct HttpProbeSummary {
    pub(super) latency: LatencySummary,
    pub(super) body_bytes: BodySizeSummary,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct BodySizeSummary {
    pub(super) p50: usize,
    pub(super) p95: usize,
    pub(super) max: usize,
}

#[derive(Debug)]
pub(super) struct DashboardProbeSamples {
    pub(super) overview: Vec<HttpProbeSample>,
    pub(super) nodes: Vec<HttpProbeSample>,
}

pub(super) async fn probe_overview_latencies(
    addr: SocketAddr,
    samples: usize,
) -> Result<Vec<Duration>> {
    Ok(probe_overview(addr, samples)
        .await?
        .into_iter()
        .map(|sample| sample.latency)
        .collect())
}

pub(super) async fn probe_overview(
    addr: SocketAddr,
    samples: usize,
) -> Result<Vec<HttpProbeSample>> {
    probe_endpoint(addr, "/api/overview", samples, 25, validate_overview_body).await
}

pub(super) async fn probe_nodes_latencies(
    addr: SocketAddr,
    samples: usize,
    expected_nodes: usize,
) -> Result<Vec<Duration>> {
    Ok(probe_nodes(addr, samples, expected_nodes)
        .await?
        .into_iter()
        .map(|sample| sample.latency)
        .collect())
}

pub(super) async fn probe_nodes(
    addr: SocketAddr,
    samples: usize,
    expected_nodes: usize,
) -> Result<Vec<HttpProbeSample>> {
    probe_endpoint(addr, "/api/nodes", samples, 20, |body| {
        validate_nodes_body(body, expected_nodes)
    })
    .await
}

pub(super) async fn probe_node_status_latencies(
    addr: SocketAddr,
    node_id: String,
    samples: usize,
) -> Result<Vec<Duration>> {
    let path = format!("/api/nodes/{node_id}");
    Ok(probe_endpoint(addr, &path, samples, 20, |body| {
        validate_node_status_body(body, &node_id)
    })
    .await?
    .into_iter()
    .map(|sample| sample.latency)
    .collect())
}

pub(super) async fn probe_node_history_latencies(
    addr: SocketAddr,
    node_id: String,
    samples: usize,
    min_points: usize,
    start_at: i64,
    end_at: i64,
) -> Result<Vec<Duration>> {
    // Use the seeded exact range so the benchmark measures a full history payload
    // instead of a heavily bucketed 24h window.
    let path = format!("/api/nodes/{node_id}/history?start={start_at}&end={end_at}&max_points=480");
    Ok(probe_endpoint(addr, &path, samples, 20, |body| {
        validate_history_body(body, &node_id, min_points)
    })
    .await?
    .into_iter()
    .map(|sample| sample.latency)
    .collect())
}

pub(super) async fn probe_node_history(
    addr: SocketAddr,
    node_id: String,
    samples: usize,
    min_points: usize,
    start_at: i64,
    end_at: i64,
) -> Result<Vec<HttpProbeSample>> {
    let path = format!("/api/nodes/{node_id}/history?start={start_at}&end={end_at}&max_points=480");
    probe_endpoint(addr, &path, samples, 20, |body| {
        validate_history_body(body, &node_id, min_points)
    })
    .await
}

pub(super) async fn probe_metrics(
    addr: SocketAddr,
    samples: usize,
) -> Result<Vec<HttpProbeSample>> {
    probe_endpoint(addr, "/metrics", samples, 50, validate_metrics_body).await
}

pub(super) async fn probe_overview_until(
    addr: SocketAddr,
    deadline: Instant,
    inter_request_delay: Duration,
) -> Result<Vec<HttpProbeSample>> {
    let mut samples = Vec::new();
    while Instant::now() < deadline {
        let (latency, body) = fetch_http_latency(addr, "/api/overview").await?;
        validate_overview_body(&body)?;
        samples.push(HttpProbeSample {
            latency,
            body_bytes: body.len(),
        });
        if !inter_request_delay.is_zero() {
            sleep(inter_request_delay).await;
        }
    }
    Ok(samples)
}

pub(super) async fn probe_nodes_until(
    addr: SocketAddr,
    deadline: Instant,
    inter_request_delay: Duration,
    expected_nodes: usize,
) -> Result<Vec<HttpProbeSample>> {
    let mut samples = Vec::new();
    while Instant::now() < deadline {
        let (latency, body) = fetch_http_latency(addr, "/api/nodes").await?;
        validate_nodes_body(&body, expected_nodes)?;
        samples.push(HttpProbeSample {
            latency,
            body_bytes: body.len(),
        });
        if !inter_request_delay.is_zero() {
            sleep(inter_request_delay).await;
        }
    }
    Ok(samples)
}

pub(super) async fn probe_dashboard_refreshes(
    addr: SocketAddr,
    samples: usize,
    expected_nodes: usize,
) -> Result<DashboardProbeSamples> {
    let mut overview = Vec::with_capacity(samples);
    let mut nodes = Vec::with_capacity(samples);
    for _ in 0..samples {
        let (overview_latency, overview_body) = fetch_http_latency(addr, "/api/overview").await?;
        validate_overview_body(&overview_body)?;
        overview.push(HttpProbeSample {
            latency: overview_latency,
            body_bytes: overview_body.len(),
        });

        let (nodes_latency, nodes_body) = fetch_http_latency(addr, "/api/nodes").await?;
        validate_nodes_body(&nodes_body, expected_nodes)?;
        nodes.push(HttpProbeSample {
            latency: nodes_latency,
            body_bytes: nodes_body.len(),
        });
        sleep(Duration::from_millis(50)).await;
    }
    Ok(DashboardProbeSamples { overview, nodes })
}

pub(super) fn summarize_latencies(latencies: &[Duration]) -> Result<LatencySummary> {
    if latencies.is_empty() {
        bail!("no overview latencies captured");
    }
    let mut values: Vec<f64> = latencies
        .iter()
        .map(|duration| duration.as_secs_f64() * 1000.0)
        .collect();
    values.sort_by(|left, right| left.total_cmp(right));

    let percentile = |p: f64| -> f64 {
        let index = ((values.len() - 1) as f64 * p).round() as usize;
        values[index]
    };

    Ok(LatencySummary {
        p50_ms: percentile(0.50),
        p95_ms: percentile(0.95),
        max_ms: *values.last().unwrap_or(&0.0),
    })
}

pub(super) fn summarize_probe_samples(samples: &[HttpProbeSample]) -> Result<HttpProbeSummary> {
    if samples.is_empty() {
        bail!("no http probe samples captured");
    }
    let latencies: Vec<_> = samples.iter().map(|sample| sample.latency).collect();
    let body_bytes: Vec<_> = samples.iter().map(|sample| sample.body_bytes).collect();
    Ok(HttpProbeSummary {
        latency: summarize_latencies(&latencies)?,
        body_bytes: summarize_body_sizes(&body_bytes)?,
    })
}

fn summarize_body_sizes(values: &[usize]) -> Result<BodySizeSummary> {
    if values.is_empty() {
        bail!("no body sizes captured");
    }
    let mut values = values.to_vec();
    values.sort_unstable();
    let percentile = |p: f64| -> usize {
        let index = ((values.len() - 1) as f64 * p).round() as usize;
        values[index]
    };
    Ok(BodySizeSummary {
        p50: percentile(0.50),
        p95: percentile(0.95),
        max: *values.last().unwrap_or(&0),
    })
}

async fn probe_endpoint<F>(
    addr: SocketAddr,
    path: &str,
    samples: usize,
    delay_ms: u64,
    validate: F,
) -> Result<Vec<HttpProbeSample>>
where
    F: Fn(&str) -> Result<()>,
{
    let mut results = Vec::with_capacity(samples);
    for _ in 0..samples {
        let (latency, body) = fetch_http_latency(addr, path).await?;
        validate(&body)?;
        results.push(HttpProbeSample {
            latency,
            body_bytes: body.len(),
        });
        sleep(Duration::from_millis(delay_ms)).await;
    }
    Ok(results)
}

async fn fetch_http_latency(addr: SocketAddr, path: &str) -> Result<(Duration, String)> {
    let started = Instant::now();
    let mut stream = TcpStream::connect(addr)
        .await
        .with_context(|| format!("connect http probe to {addr}"))?;
    let request = format!(
        "GET {path} HTTP/1.1\r\nHost: {addr}\r\nAuthorization: {LOAD_TEST_BASIC_AUTH}\r\nConnection: close\r\n\r\n"
    );
    stream
        .write_all(request.as_bytes())
        .await
        .with_context(|| format!("write http request for {path}"))?;

    let mut response = Vec::new();
    stream
        .read_to_end(&mut response)
        .await
        .with_context(|| format!("read http response for {path}"))?;

    let response_text = String::from_utf8_lossy(&response);
    if !response_text.starts_with("HTTP/1.1 200") && !response_text.starts_with("HTTP/1.0 200") {
        bail!("unexpected http response for {path}: {response_text}");
    }

    let Some((_, body)) = response_text.split_once("\r\n\r\n") else {
        bail!("missing http body separator for {path}");
    };

    Ok((started.elapsed(), body.to_string()))
}

fn validate_overview_body(body: &str) -> Result<()> {
    let overview: serde_json::Value = serde_json::from_str(body).context("decode overview body")?;
    let total_nodes = overview
        .get("total_nodes")
        .and_then(serde_json::Value::as_u64)
        .context("overview missing total_nodes")?;
    if total_nodes == 0 {
        bail!("overview returned zero nodes");
    }
    Ok(())
}

fn validate_metrics_body(body: &str) -> Result<()> {
    if !body.contains("nodelite_nodes_total") {
        bail!("metrics response missing nodelite_nodes_total");
    }
    if !body.contains("nodelite_history_dropped_writes_total") {
        bail!("metrics response missing history dropped writes counter");
    }
    Ok(())
}

fn validate_nodes_body(body: &str, expected_nodes: usize) -> Result<()> {
    let statuses: Vec<NodeListItem> = serde_json::from_str(body).context("decode nodes body")?;
    if statuses.len() != expected_nodes {
        bail!(
            "nodes endpoint returned {} nodes, expected {expected_nodes}",
            statuses.len()
        );
    }
    Ok(())
}

fn validate_node_status_body(body: &str, node_id: &str) -> Result<()> {
    let status: NodeStatus = serde_json::from_str(body).context("decode node status body")?;
    if status.identity.node_id != node_id {
        bail!(
            "node status endpoint returned {} instead of {node_id}",
            status.identity.node_id
        );
    }
    Ok(())
}

fn validate_history_body(body: &str, node_id: &str, min_points: usize) -> Result<()> {
    let points: Vec<nodelite_proto::HistoryPoint> =
        serde_json::from_str(body).context("decode node history body")?;
    if points.len() < min_points {
        bail!(
            "history endpoint returned only {} points for {node_id}, expected at least {min_points}",
            points.len()
        );
    }
    if points.iter().any(|point| point.node_id != node_id) {
        bail!("history endpoint mixed node ids for {node_id}");
    }
    Ok(())
}
