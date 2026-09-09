//! A session has one absolute authentication deadline and a renewable inbound deadline.

use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use futures::StreamExt;
use tokio::time::{Instant, MissedTickBehavior, interval, sleep_until, timeout, timeout_at};
use tokio_tungstenite::tungstenite::{Error as WebSocketError, Message};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async_with_config};
use tracing::{info, warn};

use nodelite_proto::{
    AgentConfig, HelloMessage, NetworkThrottleMessage, NodeIdentity, NoticeLevel, PingMessage,
    PongMessage, RefreshTokenResponseMessage, ServerNoticeMessage, WIRE_PROTOCOL_VERSION,
    WireMessage,
};

use crate::collector::HostCollector;
use crate::config_io::update_token_in_config;
use crate::traffic_control::TrafficController;

use super::transport::TimedSender;
use super::{
    AgentLogBuffer, AgentWsSender, SessionError, apply_network_throttle, flush_agent_logs,
    incoming_ws_config, log_notice, send_metrics, send_traffic_control_status, send_wire_message,
    server_notice_reports_token_expired, session_error, token_expired_error,
};

type AgentWsReceiver =
    futures::stream::SplitStream<WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>>;

enum SessionEvent {
    Incoming(Option<Result<Message, WebSocketError>>),
    Report,
    RetryThrottle(Option<u64>),
}

struct Session<'a> {
    config: &'a mut AgentConfig,
    config_path: &'a Path,
    logs: &'a mut AgentLogBuffer,
    sender: AgentWsSender,
    authenticated: bool,
    traffic: TrafficController,
}

/// 与 Server 进行一次完整会话；任何期限到达都会交还给外层重连策略。
pub async fn run_session(
    config: &mut AgentConfig,
    collector: &mut HostCollector,
    identity: &NodeIdentity,
    config_path: &Path,
    log_buffer: &mut AgentLogBuffer,
) -> Result<(), SessionError> {
    log_buffer.push(
        NoticeLevel::Info,
        format!("connecting to {}", config.server),
    );
    let (socket, _) = timeout(
        Duration::from_secs(config.connect_timeout_secs),
        connect_async_with_config(
            config.server.as_str(),
            Some(incoming_ws_config(config.max_incoming_message_bytes)),
            false,
        ),
    )
    .await
    .map_err(|_| session_error(false, anyhow!("timed out connecting to {}", config.server)))?
    .map_err(|error| {
        session_error(
            false,
            anyhow!("failed to connect to {}: {error}", config.server),
        )
    })?;
    let (sender, receiver) = socket.split();
    let mut session = Session {
        sender: TimedSender::new(sender, Duration::from_secs(config.send_timeout_secs)),
        config,
        config_path,
        logs: log_buffer,
        authenticated: false,
        traffic: TrafficController::default(),
    };
    let deadline = Instant::now() + Duration::from_secs(session.config.auth_timeout_secs);
    let hello = WireMessage::Hello(HelloMessage {
        protocol_version: WIRE_PROTOCOL_VERSION,
        token: session.config.token.clone(),
        identity: identity.clone(),
    });
    timeout_at(deadline, send_wire_message(&mut session.sender, &hello))
        .await
        .map_err(|_| session.deadline_error())?
        .map_err(|error| session_error(false, error))?;
    session.run(receiver, collector, deadline).await
}

impl Session<'_> {
    async fn run(
        &mut self,
        mut receiver: AgentWsReceiver,
        collector: &mut HostCollector,
        mut deadline: Instant,
    ) -> Result<(), SessionError> {
        let inbound_timeout = Duration::from_secs(self.config.inbound_timeout_secs);
        let mut report = interval(Duration::from_secs(self.config.report_interval_secs));
        report.set_missed_tick_behavior(MissedTickBehavior::Skip);
        loop {
            let retry = self.traffic.retry_policy();
            let event = tokio::select! {
                biased;
                _ = sleep_until(deadline) => return Err(self.deadline_error()),
                _ = report.tick(), if self.authenticated => SessionEvent::Report,
                _ = async {
                    match retry {
                        Some((when, _)) => sleep_until(when).await,
                        None => std::future::pending::<()>().await,
                    }
                }, if self.authenticated => SessionEvent::RetryThrottle(retry.and_then(|(_, rate)| rate)),
                incoming = receiver.next() => SessionEvent::Incoming(incoming),
            };
            let was_authenticated = self.authenticated;
            if was_authenticated && matches!(&event, SessionEvent::Incoming(_)) {
                deadline = Instant::now() + inbound_timeout;
            }
            timeout_at(deadline, self.process_event(event, collector))
                .await
                .map_err(|_| self.deadline_error())??;
            if !was_authenticated && self.authenticated {
                deadline = Instant::now() + inbound_timeout;
                timeout_at(deadline, self.initial_telemetry())
                    .await
                    .map_err(|_| self.deadline_error())?
                    .map_err(|error| session_error(true, error))?;
            }
        }
    }

    fn deadline_error(&self) -> SessionError {
        session_error(
            self.authenticated,
            anyhow!(if self.authenticated {
                "server inbound activity timed out"
            } else {
                "server authentication response timed out"
            }),
        )
    }

    async fn process_event(
        &mut self,
        event: SessionEvent,
        collector: &mut HostCollector,
    ) -> Result<(), SessionError> {
        match event {
            SessionEvent::Incoming(frame) => {
                let frame = frame
                    .ok_or_else(|| {
                        session_error(
                            self.authenticated,
                            anyhow!("server closed websocket connection"),
                        )
                    })?
                    .map_err(|error| session_error(self.authenticated, error.into()))?;
                self.handle_frame(frame).await
            }
            SessionEvent::Report => send_metrics(&mut self.sender, collector)
                .await
                .map_err(|error| session_error(true, error)),
            SessionEvent::RetryThrottle(rate) => self.throttle(rate).await,
        }
    }

    async fn handle_frame(&mut self, frame: Message) -> Result<(), SessionError> {
        match frame {
            Message::Text(text) => {
                let message = serde_json::from_str(&text)
                    .context("invalid websocket json")
                    .map_err(|error| session_error(self.authenticated, error))?;
                self.handle_wire(message).await
            }
            Message::Ping(payload) => self
                .sender
                .send(Message::Pong(payload))
                .await
                .context("failed to reply to ping frame")
                .map_err(|error| session_error(self.authenticated, error)),
            Message::Pong(_) => Ok(()),
            Message::Close(frame) => Err(session_error(
                self.authenticated,
                anyhow!("server closed websocket connection: {frame:?}"),
            )),
            Message::Binary(_) | Message::Frame(_) => Err(session_error(
                self.authenticated,
                anyhow!("binary websocket frames are not supported"),
            )),
        }
    }

    async fn handle_wire(&mut self, message: WireMessage) -> Result<(), SessionError> {
        match message {
            WireMessage::Ping(PingMessage { nonce }) => {
                send_wire_message(&mut self.sender, &WireMessage::Pong(PongMessage { nonce }))
                    .await
                    .map_err(|error| session_error(self.authenticated, error))
            }
            WireMessage::ServerNotice(notice) => self.notice(notice).await,
            WireMessage::RefreshTokenResponse(response) => self
                .refresh_token(response)
                .await
                .map_err(|error| session_error(self.authenticated, error)),
            WireMessage::NetworkThrottle(NetworkThrottleMessage { rate_kbps }) => {
                self.throttle(rate_kbps).await
            }
            _ => Err(session_error(
                self.authenticated,
                anyhow!("received unexpected websocket message from server"),
            )),
        }
    }

    async fn notice(&mut self, notice: ServerNoticeMessage) -> Result<(), SessionError> {
        let ServerNoticeMessage {
            level,
            code,
            message,
        } = notice;
        if !self.authenticated && matches!(level, NoticeLevel::Info) && message == "authenticated" {
            self.authenticated = true;
            self.traffic.probe();
            self.logs.push(
                NoticeLevel::Info,
                format!("authenticated with {}", self.config.server),
            );
        }
        if server_notice_reports_token_expired(level, code, &message) {
            self.logs.push(
                NoticeLevel::Error,
                "agent token expired; waiting for operator to rotate token",
            );
            tracing::error!(message = %message, "agent token expired; sleeping until operator rotates token");
            return Err(token_expired_error(anyhow!("agent token expired")));
        }
        log_notice(level, &message);
        if message != "authenticated" {
            self.logs.push(level, format!("server notice: {message}"));
            if self.authenticated {
                flush_agent_logs(&mut self.sender, self.logs)
                    .await
                    .map_err(|error| session_error(true, error))?;
            }
        }
        Ok(())
    }

    async fn initial_telemetry(&mut self) -> Result<()> {
        send_traffic_control_status(&mut self.sender, &self.traffic).await?;
        flush_agent_logs(&mut self.sender, self.logs).await
    }

    async fn refresh_token(&mut self, response: RefreshTokenResponseMessage) -> Result<()> {
        info!("received new token, expires at {}", response.expires_at);
        self.logs.push(
            NoticeLevel::Info,
            format!(
                "received refreshed token expiring at {}",
                response.expires_at
            ),
        );
        self.config.token = response.new_token.clone();
        if let Err(error) = update_token_in_config(self.config_path, &response.new_token).await {
            warn!("failed to persist new token: {}", error);
            self.logs.push(
                NoticeLevel::Warn,
                format!("failed to persist refreshed token: {error}"),
            );
        } else {
            info!("successfully persisted new token to config file");
            self.logs.push(
                NoticeLevel::Info,
                "persisted refreshed token to local config",
            );
        }
        flush_agent_logs(&mut self.sender, self.logs).await
    }

    async fn throttle(&mut self, rate: Option<u64>) -> Result<(), SessionError> {
        apply_network_throttle(
            &mut self.traffic,
            &mut self.sender,
            self.logs,
            self.authenticated,
            rate,
        )
        .await
        .map_err(|error| session_error(self.authenticated, error))
    }
}
