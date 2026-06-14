//! 已认证 WebSocket 会话循环。

use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow};
use axum::extract::ws::{CloseFrame, Message, WebSocket, close_code};
use futures::{SinkExt, StreamExt};
use nodelite_proto::{
    AgentLogEntry, AgentLogsMessage, MetricsMessage, NodeSnapshot, PongMessage,
    RefreshTokenRequestMessage, ServerNoticeMessage, WireMessage,
};
use tokio::time::{MissedTickBehavior, interval};
use tracing::{info, warn};

use super::protocol::{
    ParsedFrame, encode_ping_message, parse_wire_message, prune_outstanding_pings,
};
use super::refresh::{
    ensure_current_token, handle_refresh_request, refresh_session_token, should_refresh_agent_token,
};
use super::{ActiveSession, LoopAction};
use crate::AppState;
use crate::sanitize::{
    METRIC_ANOMALY_SESSION_LIMIT, METRIC_ANOMALY_WINDOW_SECS, sanitize_snapshot,
    should_disconnect_for_metric_anomalies, update_metric_anomaly_window,
};
use crate::state::{SessionCommand, SessionRefreshReply};

pub(crate) struct SessionLoopState {
    ping_ticker: tokio::time::Interval,
    ping_expiry: Duration,
    outstanding_pings: HashMap<u64, Instant>,
    next_ping_nonce: u64,
    metric_anomaly_window: VecDeque<Instant>,
}

impl SessionLoopState {
    pub(crate) fn new(ping_interval_secs: u64) -> Self {
        let ping_every = Duration::from_secs(ping_interval_secs);
        let ping_expiry = Duration::from_secs(ping_interval_secs.saturating_mul(3));
        let mut ping_ticker = interval(ping_every);
        ping_ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
        Self {
            ping_ticker,
            ping_expiry,
            outstanding_pings: HashMap::new(),
            next_ping_nonce: 1,
            metric_anomaly_window: VecDeque::new(),
        }
    }
}

pub(crate) async fn run_authenticated_session(
    state: &AppState,
    socket: WebSocket,
    session: &mut ActiveSession,
) -> Result<(), super::ProtocolError> {
    let shared = state.shared.clone();
    let (mut sender, mut receiver) = socket.split();
    let (session_control, mut session_control_rx) = crate::state::SessionControlHandle::channel();
    if !shared
        .attach_session_control(&session.node_id, session.session_id, session_control)
        .await
    {
        warn!(
            node_id = %session.node_id,
            session_id = session.session_id,
            "failed to attach control channel for superseded session"
        );
        return Ok(());
    }

    let notice = WireMessage::ServerNotice(ServerNoticeMessage {
        level: nodelite_proto::NoticeLevel::Info,
        message: "authenticated".to_string(),
    });
    let payload = serde_json::to_string(&notice)
        .map_err(|error| anyhow!("failed to serialize authenticated notice: {error}"))?;
    sender
        .send(Message::Text(payload.into()))
        .await
        .map_err(|error| anyhow!("failed to send authenticated notice: {error}"))?;

    let mut loop_state = SessionLoopState::new(shared.config().ping_interval_secs);
    loop {
        tokio::select! {
            biased;
            _ = state.shutdown.cancelled() => {
                info!(node_id = %session.node_id, session_id = session.session_id, "closing websocket session due to server shutdown");
                let _ = sender
                    .send(Message::Close(Some(CloseFrame {
                        code: close_code::AWAY,
                        reason: "server shutting down".into(),
                    })))
                    .await;
                return Ok(());
            }
            incoming = receiver.next() => {
                let Some(frame) = incoming else {
                    return Ok(());
                };
                let frame = frame.map_err(|error| anyhow!("websocket receive failed: {error}"))?;
                if matches!(
                    handle_incoming_frame(state, &shared, session, &mut sender, &mut loop_state, frame).await?,
                    LoopAction::Break
                ) {
                    return Ok(());
                }
            }
            command = session_control_rx.recv() => {
                let Some(command) = command else {
                    return Ok(());
                };
                if matches!(
                    handle_session_command(state, session, &mut sender, command).await?,
                    LoopAction::Break
                ) {
                    return Ok(());
                }
            }
            _ = loop_state.ping_ticker.tick() => {
                if matches!(
                    handle_ping_tick(state, &shared, session, &mut sender, &mut loop_state).await?,
                    LoopAction::Break
                ) {
                    return Ok(());
                }
            }
        }
    }
}

async fn handle_incoming_frame(
    state: &AppState,
    shared: &crate::state::SharedState,
    session: &mut ActiveSession,
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    loop_state: &mut SessionLoopState,
    frame: Message,
) -> Result<LoopAction, super::ProtocolError> {
    match parse_wire_message(frame)? {
        ParsedFrame::Close => Ok(LoopAction::Break),
        ParsedFrame::Control => Ok(LoopAction::Continue),
        ParsedFrame::Wire(message) => {
            handle_wire_message(state, shared, session, sender, loop_state, *message).await
        }
    }
}

async fn handle_wire_message(
    state: &AppState,
    shared: &crate::state::SharedState,
    session: &mut ActiveSession,
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    loop_state: &mut SessionLoopState,
    message: WireMessage,
) -> Result<LoopAction, super::ProtocolError> {
    match classify_agent_wire_message(message) {
        AgentWireAction::Metrics(snapshot) => {
            shared.record_ws_metrics_message();
            handle_metrics_message(state, shared, session, loop_state, snapshot).await
        }
        AgentWireAction::AgentLogs(entries) => {
            shared.record_ws_agent_logs_message();
            handle_agent_logs_message(state, session, entries).await
        }
        AgentWireAction::Pong(nonce) => {
            shared.record_ws_pong_message();
            handle_pong_message(shared, state, session, loop_state, nonce).await
        }
        AgentWireAction::RefreshTokenRequest(request) => {
            shared.record_ws_refresh_token_request_message();
            handle_refresh_request(state, session, sender, request).await
        }
        AgentWireAction::Reject(message) => Err(super::ProtocolError::Client(message.to_string())),
    }
}

#[derive(Debug)]
enum AgentWireAction {
    Metrics(NodeSnapshot),
    AgentLogs(Vec<AgentLogEntry>),
    Pong(u64),
    RefreshTokenRequest(RefreshTokenRequestMessage),
    Reject(&'static str),
}

fn classify_agent_wire_message(message: WireMessage) -> AgentWireAction {
    match message {
        WireMessage::Metrics(MetricsMessage { snapshot }) => AgentWireAction::Metrics(snapshot),
        WireMessage::AgentLogs(AgentLogsMessage { entries }) => AgentWireAction::AgentLogs(entries),
        WireMessage::Pong(PongMessage { nonce }) => AgentWireAction::Pong(nonce),
        WireMessage::RefreshTokenRequest(request) => AgentWireAction::RefreshTokenRequest(request),
        WireMessage::Hello(_) => AgentWireAction::Reject("duplicate hello message"),
        WireMessage::Ping(_) => AgentWireAction::Reject("agent must not send ping messages"),
        WireMessage::ServerNotice(_) => {
            AgentWireAction::Reject("agent must not send server_notice messages")
        }
        WireMessage::RefreshTokenResponse(_) => {
            AgentWireAction::Reject("agent must not send refresh_token_response messages")
        }
    }
}

async fn handle_metrics_message(
    state: &AppState,
    shared: &crate::state::SharedState,
    session: &mut ActiveSession,
    loop_state: &mut SessionLoopState,
    snapshot: nodelite_proto::NodeSnapshot,
) -> Result<LoopAction, super::ProtocolError> {
    if !ensure_current_token(
        state,
        session,
        "disconnecting session after registry token change",
    )
    .await
    {
        return Ok(LoopAction::Break);
    }
    let (snapshot, report) = sanitize_snapshot(shared.config(), snapshot);
    if report.modified() {
        update_metric_anomaly_window(
            &mut loop_state.metric_anomaly_window,
            &report,
            Instant::now(),
        );
        warn!(
            node_id = %session.node_id,
            session_id = session.session_id,
            anomalies = report.total(),
            anomaly_window_size = loop_state.metric_anomaly_window.len(),
            "agent reported out-of-range metrics; clamped before persistence",
        );
        if should_disconnect_for_metric_anomalies(&loop_state.metric_anomaly_window) {
            warn!(
                node_id = %session.node_id,
                session_id = session.session_id,
                limit = METRIC_ANOMALY_SESSION_LIMIT,
                window_secs = METRIC_ANOMALY_WINDOW_SECS,
                "disconnecting session after repeated metric anomalies",
            );
            return Ok(LoopAction::Break);
        }
    }
    let Some(status) = shared
        .update_snapshot(&session.node_id, session.session_id, snapshot)
        .await
    else {
        warn!(
            node_id = %session.node_id,
            session_id = session.session_id,
            "dropping metrics from superseded session"
        );
        return Ok(LoopAction::Break);
    };
    state.history.record_status(&status).await;
    Ok(LoopAction::Continue)
}

async fn handle_agent_logs_message(
    state: &AppState,
    session: &mut ActiveSession,
    entries: Vec<nodelite_proto::AgentLogEntry>,
) -> Result<LoopAction, super::ProtocolError> {
    if !ensure_current_token(
        state,
        session,
        "disconnecting session after registry token change",
    )
    .await
    {
        return Ok(LoopAction::Break);
    }
    let result = state
        .agent_logs
        .record_entries(&session.node_id, entries)
        .await;
    if result.accepted > 0 {
        info!(node_id = %session.node_id, accepted = result.accepted, "recorded agent runtime log entries");
    }
    if result.total_dropped() > 0 {
        warn!(
            node_id = %session.node_id,
            accepted = result.accepted,
            dropped_batch_cap = result.dropped_batch_cap,
            dropped_sanitize = result.dropped_sanitize,
            evicted_global_budget = result.evicted_global_budget,
            "dropped some agent log entries during ingestion"
        );
    }
    Ok(LoopAction::Continue)
}

async fn handle_pong_message(
    shared: &crate::state::SharedState,
    state: &AppState,
    session: &mut ActiveSession,
    loop_state: &mut SessionLoopState,
    nonce: u64,
) -> Result<LoopAction, super::ProtocolError> {
    if !ensure_current_token(
        state,
        session,
        "disconnecting session after registry token change",
    )
    .await
    {
        return Ok(LoopAction::Break);
    }
    let Some(latency_ms) = consume_pong_latency(loop_state, nonce, Instant::now()) else {
        return Ok(LoopAction::Continue);
    };
    if !shared
        .update_latency(&session.node_id, session.session_id, latency_ms)
        .await
    {
        warn!(
            node_id = %session.node_id,
            session_id = session.session_id,
            "dropping pong from superseded session"
        );
        return Ok(LoopAction::Break);
    }
    Ok(LoopAction::Continue)
}

fn consume_pong_latency(
    loop_state: &mut SessionLoopState,
    nonce: u64,
    now: Instant,
) -> Option<u64> {
    loop_state
        .outstanding_pings
        .remove(&nonce)
        .map(|sent_at| now.duration_since(sent_at).as_millis() as u64)
}

async fn handle_session_command(
    state: &AppState,
    session: &mut ActiveSession,
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    command: SessionCommand,
) -> Result<LoopAction, super::ProtocolError> {
    match command {
        SessionCommand::RefreshToken {
            response,
            refresh_permit: _refresh_permit,
        } => match refresh_session_token(sender, &state.registry, session, "manual").await {
            Ok(expires_at) => {
                let _ = response.send(Ok(SessionRefreshReply {
                    token_expires_at: expires_at,
                }));
                Ok(LoopAction::Continue)
            }
            Err(error) => {
                let message = error.to_string();
                let _ = response.send(Err(message));
                Err(super::ProtocolError::Server(error))
            }
        },
    }
}

async fn handle_ping_tick(
    state: &AppState,
    shared: &crate::state::SharedState,
    session: &mut ActiveSession,
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    loop_state: &mut SessionLoopState,
) -> Result<LoopAction, super::ProtocolError> {
    if !shared
        .is_current_session(&session.node_id, session.session_id)
        .await
    {
        warn!(
            node_id = %session.node_id,
            session_id = session.session_id,
            "closing superseded websocket session"
        );
        return Ok(LoopAction::Break);
    }
    if !ensure_current_token(
        state,
        session,
        "closing websocket session after registry token change",
    )
    .await
    {
        return Ok(LoopAction::Break);
    }
    if should_refresh_agent_token(&state.registry, session).await? {
        refresh_session_token(sender, &state.registry, session, "pre-expiry").await?;
    }

    prune_outstanding_pings(
        &mut loop_state.outstanding_pings,
        loop_state.ping_expiry,
        shared.config().max_outstanding_pings,
    );
    let nonce = loop_state.next_ping_nonce;
    loop_state.next_ping_nonce = loop_state.next_ping_nonce.saturating_add(1);
    loop_state.outstanding_pings.insert(nonce, Instant::now());
    let ping = encode_ping_message(nonce)?;
    sender
        .send(Message::Text(ping.into()))
        .await
        .map_err(|error| anyhow!("failed to send ping: {error}"))?;
    Ok(LoopAction::Continue)
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use axum::extract::ws::Message;
    use nodelite_proto::{
        HelloMessage, NodeIdentity, NoticeLevel, PingMessage, PongMessage,
        RefreshTokenResponseMessage, ServerNoticeMessage, WIRE_PROTOCOL_VERSION, WireMessage,
    };

    use super::{
        AgentWireAction, SessionLoopState, classify_agent_wire_message, consume_pong_latency,
    };

    fn identity() -> NodeIdentity {
        NodeIdentity {
            node_id: "hk-01".to_string(),
            node_label: "Hong Kong 01".to_string(),
            hostname: "hk-01.internal".to_string(),
            os: "Linux".to_string(),
            kernel_version: None,
            cpu_model: None,
            cpu_cores: 2,
            agent_version: "0.1.0-test".to_string(),
            boot_time: None,
            tags: Vec::new(),
        }
    }

    fn hello_message() -> HelloMessage {
        HelloMessage {
            protocol_version: WIRE_PROTOCOL_VERSION,
            identity: identity(),
            token: "secret".to_string(),
        }
    }

    #[tokio::test]
    async fn session_loop_state_sets_ping_expiry_from_interval() {
        let state = SessionLoopState::new(7);

        assert_eq!(state.ping_expiry, Duration::from_secs(21));
        assert_eq!(state.next_ping_nonce, 1);
        assert!(state.outstanding_pings.is_empty());
    }

    #[test]
    fn classify_routes_agent_business_messages() {
        let pong = classify_agent_wire_message(WireMessage::Pong(PongMessage { nonce: 42 }));

        assert!(matches!(pong, AgentWireAction::Pong(42)));
    }

    #[test]
    fn classify_rejects_agent_forbidden_messages() {
        let cases = [
            (
                WireMessage::Hello(hello_message()),
                "duplicate hello message",
            ),
            (
                WireMessage::Ping(PingMessage { nonce: 1 }),
                "agent must not send ping messages",
            ),
            (
                WireMessage::ServerNotice(ServerNoticeMessage {
                    level: NoticeLevel::Info,
                    message: "server-only".to_string(),
                }),
                "agent must not send server_notice messages",
            ),
            (
                WireMessage::RefreshTokenResponse(RefreshTokenResponseMessage {
                    new_token: "new-token".to_string(),
                    expires_at: "2026-01-01T00:00:00Z".to_string(),
                }),
                "agent must not send refresh_token_response messages",
            ),
        ];

        for (message, expected) in cases {
            let action = classify_agent_wire_message(message);
            assert!(matches!(action, AgentWireAction::Reject(actual) if actual == expected));
        }
    }

    #[tokio::test]
    async fn consume_pong_latency_removes_known_nonce() {
        let mut state = SessionLoopState::new(10);
        let now = Instant::now();
        state
            .outstanding_pings
            .insert(7, now - Duration::from_millis(42));

        let latency = consume_pong_latency(&mut state, 7, now);

        assert_eq!(latency, Some(42));
        assert!(state.outstanding_pings.is_empty());
    }

    #[tokio::test]
    async fn consume_pong_latency_ignores_unknown_nonce() {
        let mut state = SessionLoopState::new(10);
        state.outstanding_pings.insert(7, Instant::now());

        let latency = consume_pong_latency(&mut state, 8, Instant::now());

        assert_eq!(latency, None);
        assert!(state.outstanding_pings.contains_key(&7));
    }

    #[test]
    fn close_frames_break_before_wire_classification() {
        assert!(matches!(
            super::super::protocol::parse_wire_message(Message::Close(None)),
            Ok(super::super::protocol::ParsedFrame::Close)
        ));
    }
}
