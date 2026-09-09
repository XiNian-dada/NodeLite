//! Reconnect and buffered log regressions.

use std::collections::HashSet;
use std::time::Duration;

use nodelite_proto::{NoticeLevel, ServerNoticeCode};

use super::{
    AgentLogBuffer, MAX_PENDING_AGENT_LOGS, SessionError, reconnect_delay, retry_log_message,
    server_notice_reports_token_expired, token_expired_reconnect_delay,
};

#[test]
fn reconnect_delay_is_within_jitter_window_and_disperses() {
    let cases: &[(u32, u64, u64)] = &[
        (0, 1, 5),
        (1, 2, 10),
        (2, 5, 20),
        (3, 10, 40),
        (4, 15, 60),
        (5, 30, 120),
        (1024, 30, 120),
    ];
    for &(attempt, floor_secs, ceiling_secs) in cases {
        let lower = Duration::from_secs(floor_secs);
        let upper = Duration::from_secs(ceiling_secs);
        let mut samples: HashSet<u128> = HashSet::new();
        for _ in 0..32 {
            let delay = reconnect_delay(attempt);
            assert!(
                delay >= lower && delay <= upper,
                "attempt {attempt}: {delay:?} not in [{lower:?}, {upper:?}]",
            );
            samples.insert(delay.as_millis());
        }
        assert!(
            samples.len() > 1,
            "attempt {attempt}: 32 samples all identical, jitter not active",
        );
    }
}

#[test]
fn token_expired_reconnect_delay_uses_short_probes_before_long_sleep() {
    let cases = [
        (0, Duration::from_secs(30)),
        (1, Duration::from_secs(120)),
        (2, Duration::from_secs(300)),
        (3, Duration::from_secs(3600)),
        (1024, Duration::from_secs(3600)),
    ];
    for (attempt, expected) in cases {
        assert_eq!(token_expired_reconnect_delay(attempt), expected);
    }
}

#[test]
fn retry_log_message_distinguishes_confirmed_token_expiry() {
    let token_error = SessionError {
        established_session: false,
        token_expired: true,
        source: anyhow::anyhow!("agent token expired"),
    };
    let short_message = retry_log_message(
        &token_error,
        "agent token expired",
        Duration::from_secs(30),
        0,
    );
    assert!(short_message.contains("confirmed token expiry"));
    assert!(short_message.contains("probing for a rotated token"));

    let long_message = retry_log_message(
        &token_error,
        "agent token expired",
        Duration::from_secs(3600),
        3,
    );
    assert!(long_message.contains("operator token rotation likely required"));

    let refresh_error = SessionError {
        established_session: true,
        token_expired: false,
        source: anyhow::anyhow!("failed to send token refresh response"),
    };
    let refresh_message = retry_log_message(
        &refresh_error,
        "failed to send token refresh response",
        Duration::from_secs(5),
        0,
    );
    assert!(refresh_message.contains("session ended after authentication"));
    assert!(!refresh_message.contains("confirmed token expiry"));
}

#[test]
fn token_expiry_notice_prefers_structured_code() {
    assert!(server_notice_reports_token_expired(
        NoticeLevel::Error,
        Some(ServerNoticeCode::TokenExpired),
        "localized operator-facing message",
    ));
    assert!(!server_notice_reports_token_expired(
        NoticeLevel::Error,
        Some(ServerNoticeCode::Unauthorized),
        "token expired",
    ));
    assert!(server_notice_reports_token_expired(
        NoticeLevel::Error,
        None,
        "token expired; rotate it",
    ));
    assert!(!server_notice_reports_token_expired(
        NoticeLevel::Warn,
        Some(ServerNoticeCode::TokenExpired),
        "token expired",
    ));
}

#[test]
fn agent_log_buffer_keeps_recent_entries() {
    let mut buffer = AgentLogBuffer::default();
    for index in 0..(MAX_PENDING_AGENT_LOGS + 4) {
        buffer.push(NoticeLevel::Info, format!("entry-{index}"));
    }
    let batch = buffer.peek_batch();
    assert_eq!(batch.len(), 32);
    assert_eq!(
        buffer.entries.front().map(|entry| entry.message.as_str()),
        Some("entry-4")
    );
}

#[test]
fn agent_log_buffer_trims_existing_overflow_in_one_pass() {
    let mut buffer = AgentLogBuffer::default();
    for index in 0..(MAX_PENDING_AGENT_LOGS * 4) {
        buffer.entries.push_back(nodelite_proto::AgentLogEntry {
            occurred_at: "2026-05-23T00:00:00Z".to_string(),
            level: NoticeLevel::Info,
            message: format!("entry-{index}"),
        });
    }

    buffer.push(NoticeLevel::Warn, "after-overflow");

    assert_eq!(buffer.entries.len(), MAX_PENDING_AGENT_LOGS);
    assert_eq!(
        buffer.entries.front().map(|entry| entry.message.as_str()),
        Some("entry-769")
    );
    assert_eq!(
        buffer.entries.back().map(|entry| entry.message.as_str()),
        Some("after-overflow")
    );
}
