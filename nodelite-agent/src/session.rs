//! Agent session lifecycle, reconnect policy and bounded runtime log forwarding.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result};
use chrono::Utc;
use getrandom::fill as fill_random;
use thiserror::Error;
use tokio::time::sleep;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
use tracing::{info, warn};

use nodelite_proto::{
    AgentConfig, AgentLogEntry, AgentLogsMessage, MetricsMessage, NoticeLevel, ServerNoticeCode,
    WireMessage, truncate_to_byte_boundary,
};

use crate::collector::{HostCollector, collect_snapshot_blocking};
use crate::traffic_control::{TrafficControlOutcome, TrafficController};

mod connection;
mod transport;

pub use connection::run_session;
use transport::TimedSender;

/// Agent 本地最多暂存的待上报日志条数。超出后丢弃最旧项,避免断线期间内存无限增长。
const MAX_PENDING_AGENT_LOGS: usize = 256;
/// 单次推送到服务端的最大日志条数,控制消息体积。
const MAX_AGENT_LOG_BATCH: usize = 32;
/// 单条日志消息的最大字节数,避免异常长错误串撑爆 WebSocket 消息。
const MAX_AGENT_LOG_MESSAGE_BYTES: usize = 240;
const TOKEN_EXPIRED_SHORT_RETRY_DELAYS: [Duration; 3] = [
    Duration::from_secs(30),
    Duration::from_secs(120),
    Duration::from_secs(300),
];
/// Token 连续确认过期后,退回长间隔以避免长期热重试。
const TOKEN_EXPIRED_LONG_RECONNECT_DELAY: Duration = Duration::from_secs(3600);

#[derive(Debug, Error)]
#[error("{source}")]
pub struct SessionError {
    /// 是否曾经成功完成认证。外部测试据此区分"连接前失败"与"连接后断开"。
    pub established_session: bool,
    pub(crate) token_expired: bool,
    pub(crate) source: anyhow::Error,
}

type AgentWsSender = TimedSender<
    futures::stream::SplitSink<
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
        Message,
    >,
>;

#[derive(Default)]
pub struct AgentLogBuffer {
    entries: VecDeque<AgentLogEntry>,
}

impl AgentLogBuffer {
    pub(crate) fn push(&mut self, level: NoticeLevel, message: impl Into<String>) {
        let message = truncate_to_byte_boundary(&message.into(), MAX_AGENT_LOG_MESSAGE_BYTES)
            .trim()
            .to_string();
        if message.is_empty() {
            return;
        }
        self.entries.push_back(AgentLogEntry {
            occurred_at: Utc::now().to_rfc3339(),
            level,
            message,
        });
        self.trim_overflow();
    }

    fn peek_batch(&self) -> Vec<AgentLogEntry> {
        self.entries
            .iter()
            .take(MAX_AGENT_LOG_BATCH)
            .cloned()
            .collect()
    }

    fn discard_sent(&mut self, count: usize) {
        for _ in 0..count {
            self.entries.pop_front();
        }
    }

    fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    fn trim_overflow(&mut self) {
        let overflow = self.entries.len().saturating_sub(MAX_PENDING_AGENT_LOGS);
        if overflow > 0 {
            self.entries.drain(..overflow);
        }
    }
}

pub async fn run_forever<F>(
    mut config: AgentConfig,
    mut collector: HostCollector,
    identity: nodelite_proto::NodeIdentity,
    config_path: PathBuf,
    mut log_buffer: AgentLogBuffer,
    shutdown: F,
) -> Result<()>
where
    F: std::future::Future<Output = ()> + Send,
{
    let mut reconnect_attempt = 0_u32;
    let mut token_expired_attempt = 0_u32;

    tokio::pin!(shutdown);

    loop {
        let next = async {
            match run_session(
                &mut config,
                &mut collector,
                &identity,
                &config_path,
                &mut log_buffer,
            )
            .await
            {
                Ok(()) => {
                    reconnect_attempt = 0;
                    token_expired_attempt = 0;
                }
                Err(error) => {
                    if error.established_session {
                        reconnect_attempt = 0;
                        token_expired_attempt = 0;
                    }
                    let delay = if error.token_expired {
                        reconnect_attempt = 0;
                        token_expired_reconnect_delay(token_expired_attempt)
                    } else {
                        token_expired_attempt = 0;
                        reconnect_delay(reconnect_attempt)
                    };
                    let reason = error.source.to_string();
                    let level = if error.token_expired {
                        NoticeLevel::Error
                    } else if error.established_session {
                        NoticeLevel::Warn
                    } else {
                        NoticeLevel::Info
                    };
                    log_buffer.push(
                        level,
                        retry_log_message(&error, &reason, delay, token_expired_attempt),
                    );
                    warn!(
                        server = %config.server,
                        delay_secs = delay.as_secs(),
                        established_session = error.established_session,
                        token_expired = error.token_expired,
                        token_expired_attempt,
                        error = ?error.source,
                        "agent session ended; retrying after backoff"
                    );
                    sleep(delay).await;
                    if error.token_expired {
                        token_expired_attempt = token_expired_attempt.saturating_add(1);
                    } else {
                        reconnect_attempt = reconnect_attempt.saturating_add(1);
                    }
                }
            }
        };

        tokio::select! {
            _ = next => continue,
            _ = &mut shutdown => {
                info!("agent shutting down");
                return Ok(());
            }
        }
    }
}

async fn apply_network_throttle(
    controller: &mut TrafficController,
    sender: &mut AgentWsSender,
    log_buffer: &mut AgentLogBuffer,
    authenticated: bool,
    rate_kbps: Option<u64>,
) -> Result<()> {
    match controller.apply(rate_kbps).await {
        Ok(TrafficControlOutcome::Applied) => {
            if let Some(rate_kbps) = rate_kbps {
                info!(rate_kbps, "applied server-requested network traffic limit");
                log_buffer.push(
                    NoticeLevel::Info,
                    format!("applied server-requested network traffic limit: {rate_kbps} kbit/s"),
                );
            } else {
                info!("cleared server-requested network traffic limit");
            }
        }
        Ok(TrafficControlOutcome::Unavailable) => {
            warn!("network traffic control is unavailable; check Agent capability status");
            log_buffer.push(
                NoticeLevel::Warn,
                "network traffic control is unavailable; check Agent capability status",
            );
        }
        Err(error) => {
            warn!(error = ?error, "failed to apply server-requested network traffic limit");
            log_buffer.push(
                NoticeLevel::Warn,
                format!("failed to apply server-requested network traffic limit: {error}"),
            );
        }
    }
    if authenticated {
        send_traffic_control_status(sender, controller).await?;
    }
    if authenticated && !log_buffer.is_empty() {
        flush_agent_logs(sender, log_buffer).await?;
    }
    Ok(())
}

async fn send_traffic_control_status(
    sender: &mut AgentWsSender,
    controller: &TrafficController,
) -> Result<()> {
    send_wire_message(
        sender,
        &WireMessage::AgentLogs(AgentLogsMessage {
            entries: Vec::new(),
            traffic_control: controller.status(),
        }),
    )
    .await
}

fn session_error(established_session: bool, source: anyhow::Error) -> SessionError {
    SessionError {
        established_session,
        token_expired: false,
        source,
    }
}

fn token_expired_error(source: anyhow::Error) -> SessionError {
    SessionError {
        established_session: false,
        token_expired: true,
        source,
    }
}

/// 构造接收侧的 WebSocket 配置。
fn incoming_ws_config(max_incoming_message_bytes: usize) -> WebSocketConfig {
    WebSocketConfig::default()
        .max_frame_size(Some(max_incoming_message_bytes))
        .max_message_size(Some(max_incoming_message_bytes))
}

async fn send_metrics(sender: &mut AgentWsSender, collector: &mut HostCollector) -> Result<()> {
    let snapshot = collect_snapshot_blocking(collector).await?;
    send_wire_message(sender, &WireMessage::Metrics(MetricsMessage { snapshot })).await
}

async fn send_wire_message(sender: &mut AgentWsSender, message: &WireMessage) -> Result<()> {
    let payload = serde_json::to_string(message).context("serialize websocket message")?;
    sender
        .send(Message::Text(payload.into()))
        .await
        .context("send websocket message")?;
    Ok(())
}

async fn flush_agent_logs(
    sender: &mut AgentWsSender,
    log_buffer: &mut AgentLogBuffer,
) -> Result<()> {
    while !log_buffer.is_empty() {
        let batch = log_buffer.peek_batch();
        if batch.is_empty() {
            break;
        }
        send_wire_message(
            sender,
            &WireMessage::AgentLogs(AgentLogsMessage {
                traffic_control: None,
                entries: batch.clone(),
            }),
        )
        .await?;
        log_buffer.discard_sent(batch.len());
    }
    Ok(())
}

fn log_notice(level: NoticeLevel, message: &str) {
    match level {
        NoticeLevel::Info => info!(message = %message, "server notice"),
        NoticeLevel::Warn => tracing::warn!(message = %message, "server notice"),
        NoticeLevel::Error => tracing::error!(message = %message, "server notice"),
    }
}

fn reconnect_delay(attempt: u32) -> Duration {
    let (floor_secs, ceiling_secs): (u64, u64) = match attempt {
        0 => (1, 5),
        1 => (2, 10),
        2 => (5, 20),
        3 => (10, 40),
        4 => (15, 60),
        _ => (30, 120),
    };
    let floor_ms = floor_secs.saturating_mul(1000);
    let ceiling_ms = ceiling_secs.saturating_mul(1000);
    let span_ms = ceiling_ms.saturating_sub(floor_ms);
    let jitter_ms = sample_random_u64()
        .map(|value| value % span_ms.saturating_add(1))
        .unwrap_or(span_ms);
    Duration::from_millis(floor_ms.saturating_add(jitter_ms))
}

fn token_expired_reconnect_delay(attempt: u32) -> Duration {
    TOKEN_EXPIRED_SHORT_RETRY_DELAYS
        .get(attempt as usize)
        .copied()
        .unwrap_or(TOKEN_EXPIRED_LONG_RECONNECT_DELAY)
}

fn server_notice_reports_token_expired(
    level: NoticeLevel,
    code: Option<ServerNoticeCode>,
    message: &str,
) -> bool {
    if !matches!(level, NoticeLevel::Error) {
        return false;
    }

    match code {
        Some(ServerNoticeCode::TokenExpired) => true,
        Some(_) => false,
        None => message.contains("token expired"),
    }
}

fn retry_log_message(
    error: &SessionError,
    reason: &str,
    delay: Duration,
    token_expired_attempt: u32,
) -> String {
    if !error.token_expired {
        let context = if error.established_session {
            "session ended after authentication"
        } else {
            "session ended before authentication"
        };
        return format!("{context}: {reason}; retrying in {}s", delay.as_secs());
    }

    if (token_expired_attempt as usize) < TOKEN_EXPIRED_SHORT_RETRY_DELAYS.len() {
        return format!(
            "confirmed token expiry: {reason}; probing for a rotated token in {}s",
            delay.as_secs()
        );
    }

    format!(
        "confirmed token expiry: {reason}; operator token rotation likely required; retrying in {}s",
        delay.as_secs()
    )
}

fn sample_random_u64() -> Option<u64> {
    let mut buf = [0_u8; 8];
    fill_random(&mut buf).ok()?;
    Some(u64::from_le_bytes(buf))
}

#[cfg(test)]
mod tests;
