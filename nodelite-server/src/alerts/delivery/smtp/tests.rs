use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use chrono::Utc;
use nodelite_proto::{
    AlertChannel, AlertComparator, AlertMetric, AlertRuleConfig, AlertScopeMode, AlertSeverity,
    AlertSmtpConfig, AlertSmtpTransport,
};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;

use super::{
    auth_plain_bytes, build_alert_message, build_inspection_message, dot_stuff,
    encode_auth_plain_payload, send_alert_event, send_auth_plain_command, send_smtp_with_timeout,
};
use crate::alerts::delivery::AlertDeliveryError;
use crate::alerts::evaluator::InspectionHighlight;
use crate::alerts::{AlertEvent, AlertEventKind, AlertMetricReading};

#[tokio::test]
async fn send_smtp_delivers_plain_message() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("listener should bind");
    let addr = listener.local_addr().expect("listener should expose addr");
    let server = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.expect("smtp client should connect");
        run_fake_smtp(socket).await
    });
    let config = smtp_config(addr.port());

    send_alert_event(&config, &sample_event())
        .await
        .expect("smtp should send");
    let session = server.await.expect("fake smtp should join");

    assert!(
        session
            .commands
            .iter()
            .any(|line| line == "EHLO nodelite.local")
    );
    assert!(
        session
            .commands
            .iter()
            .any(|line| line == "MAIL FROM:<ops@example.com>")
    );
    assert!(
        session
            .commands
            .iter()
            .any(|line| line == "RCPT TO:<oncall@example.com>")
    );
    assert!(
        session
            .message
            .contains("Subject: [NodeLite] triggered CPU hot on Hong Kong")
    );
    assert!(session.message.contains("Metric: CpuUsagePercent"));
}

#[tokio::test]
async fn send_smtp_uses_compatible_auth_plain_payload() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("listener should bind");
    let addr = listener.local_addr().expect("listener should expose addr");
    let server = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.expect("smtp client should connect");
        run_fake_smtp(socket).await
    });
    let mut config = smtp_config(addr.port());
    config.username = "ops@example.com".to_string();
    config.password = Some("smtp-secret".to_string());
    let expected_payload = STANDARD.encode(b"\0ops@example.com\0smtp-secret");

    send_alert_event(&config, &sample_event())
        .await
        .expect("authenticated smtp should send");
    let session = server.await.expect("fake smtp should join");

    assert!(
        session
            .commands
            .iter()
            .any(|line| line == &format!("AUTH PLAIN {expected_payload}"))
    );
}

#[test]
fn auth_plain_payload_matches_smtp_format_and_clears_raw_buffer() {
    let mut raw_auth = auth_plain_bytes("ops@example.com", "smtp-secret");

    let payload = encode_auth_plain_payload(&mut raw_auth);

    let decoded = STANDARD
        .decode(payload.as_slice())
        .expect("auth payload should decode");
    assert_eq!(decoded, b"\0ops@example.com\0smtp-secret");
    assert!(raw_auth.iter().all(|byte| *byte == 0));
}

#[tokio::test]
async fn auth_plain_payload_is_cleared_when_send_is_cancelled() {
    let payload_dropped = Arc::new(AtomicBool::new(false));
    let drop_marker = Arc::clone(&payload_dropped);

    let result = tokio::time::timeout(std::time::Duration::from_millis(50), async {
        let (mut stream, _peer) = tokio::io::duplex(1);
        let mut raw_auth = auth_plain_bytes("ops@example.com", "smtp-secret");
        let payload = encode_auth_plain_payload(&mut raw_auth).with_drop_marker(drop_marker);

        send_auth_plain_command(&mut stream, payload.as_slice())
            .await
            .expect("stalled writer should not complete before timeout");
        drop(payload);
    })
    .await;

    assert!(result.is_err());
    assert!(payload_dropped.load(Ordering::SeqCst));
}

#[tokio::test]
async fn send_smtp_reports_protocol_error() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("listener should bind");
    let addr = listener.local_addr().expect("listener should expose addr");
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.expect("smtp client should connect");
        socket
            .write_all(b"421 fake.smtp unavailable\r\n")
            .await
            .expect("error response should write");
    });
    let config = smtp_config(addr.port());

    let error = send_alert_event(&config, &sample_event())
        .await
        .expect_err("smtp protocol error should fail delivery");
    server.await.expect("fake smtp should join");

    assert!(
        matches!(error, AlertDeliveryError::Smtp(message) if message.contains("421 fake.smtp unavailable"))
    );
}

#[tokio::test]
async fn send_smtp_times_out_waiting_for_greeting() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("listener should bind");
    let addr = listener.local_addr().expect("listener should expose addr");
    let (accepted_tx, accepted_rx) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.expect("smtp client should connect");
        accepted_tx
            .send(())
            .expect("test should await accept signal");
        let _hold_open = socket;
        std::future::pending::<()>().await;
    });
    let config = smtp_config(addr.port());
    let message = build_alert_message(&config, &sample_event()).expect("message should build");
    let delivery = tokio::spawn(async move {
        send_smtp_with_timeout(&config, message, std::time::Duration::from_millis(50)).await
    });

    tokio::time::timeout(std::time::Duration::from_secs(1), accepted_rx)
        .await
        .expect("smtp server should accept connection promptly")
        .expect("smtp server should accept connection");
    let error = tokio::time::timeout(std::time::Duration::from_secs(1), delivery)
        .await
        .expect("delivery should finish after timeout")
        .expect("delivery task should join")
        .expect_err("smtp greeting should time out");
    server.abort();

    assert!(matches!(error, AlertDeliveryError::SmtpTimeout));
}

#[test]
fn build_message_rejects_header_injection() {
    let mut event = sample_event();
    event.node_label = "good\r\nBcc: bad@example.com".to_string();

    assert!(build_alert_message(&smtp_config(25), &event).is_err());
}

#[test]
fn dot_stuff_prefixes_lines_starting_with_dot() {
    assert_eq!(dot_stuff("first\n.second"), "first\r\n..second");
}

#[test]
fn build_inspection_message_includes_totals_and_highlights() {
    let report = crate::alerts::InspectionReport {
        total_nodes: 2,
        offline_nodes: 1,
        latency_nodes: 1,
        cpu_hot_nodes: 0,
        memory_hot_nodes: 0,
        highlights: vec![InspectionHighlight {
            node_id: "hk-01".to_string(),
            node_label: "Hong Kong <edge>".to_string(),
            reasons: vec!["offline".to_string(), "latency & jitter".to_string()],
        }],
    };
    let summary = super::InspectionSummary {
        occurred_at: Utc::now(),
        local_date: chrono::NaiveDate::from_ymd_opt(2026, 5, 27).expect("date should be valid"),
        lookback_hours: 24,
        report: &report,
    };

    let message =
        build_inspection_message(&smtp_config(25), &summary).expect("message should build");

    assert!(message.contains("Subject: [NodeLite] Daily inspection 2026-05-27"));
    assert!(message.contains("Content-Type: multipart/alternative"));
    assert!(message.contains("Content-Type: text/plain; charset=utf-8"));
    assert!(message.contains("Content-Type: text/html; charset=utf-8"));
    assert!(message.contains("Total nodes: 2"));
    assert!(message.contains("- Hong Kong <edge> (hk-01): offline, latency & jitter"));
    assert!(message.contains("NodeLite Daily Inspection"));
    assert!(message.contains("High latency"));
    assert!(message.contains("Hong Kong &lt;edge&gt;"));
    assert!(message.contains("latency &amp; jitter"));
}

async fn run_fake_smtp(socket: tokio::net::TcpStream) -> SmtpSession {
    let (read_half, mut write_half) = socket.into_split();
    let mut reader = BufReader::new(read_half);
    let mut commands = Vec::new();
    let mut message = String::new();

    write_half
        .write_all(b"220 fake.smtp ESMTP\r\n")
        .await
        .expect("greeting should write");
    loop {
        let mut line = String::new();
        reader
            .read_line(&mut line)
            .await
            .expect("command should read");
        let command = line.trim_end_matches(['\r', '\n']).to_string();
        commands.push(command.clone());
        if command.starts_with("EHLO ") {
            write_half
                .write_all(b"250-fake.smtp\r\n250 AUTH PLAIN\r\n")
                .await
                .expect("ehlo response should write");
        } else if command.starts_with("AUTH PLAIN ") {
            write_half
                .write_all(b"235 2.7.0 Authentication successful\r\n")
                .await
                .expect("auth response should write");
        } else if command.starts_with("MAIL FROM:") || command.starts_with("RCPT TO:") {
            write_half
                .write_all(b"250 OK\r\n")
                .await
                .expect("mail response should write");
        } else if command == "DATA" {
            write_half
                .write_all(b"354 End data with <CR><LF>.<CR><LF>\r\n")
                .await
                .expect("data response should write");
            loop {
                let mut body_line = String::new();
                reader
                    .read_line(&mut body_line)
                    .await
                    .expect("message should read");
                if body_line == ".\r\n" {
                    break;
                }
                message.push_str(&body_line);
            }
            write_half
                .write_all(b"250 Queued\r\n")
                .await
                .expect("queued response should write");
        } else if command == "QUIT" {
            write_half
                .write_all(b"221 Bye\r\n")
                .await
                .expect("quit response should write");
            break;
        } else {
            write_half
                .write_all(b"250 OK\r\n")
                .await
                .expect("generic response should write");
        }
    }

    SmtpSession { commands, message }
}

struct SmtpSession {
    commands: Vec<String>,
    message: String,
}

fn smtp_config(port: u16) -> AlertSmtpConfig {
    AlertSmtpConfig {
        enabled: true,
        host: "127.0.0.1".to_string(),
        port,
        username: String::new(),
        password: None,
        sender: "ops@example.com".to_string(),
        recipients: vec!["oncall@example.com".to_string()],
        transport: AlertSmtpTransport::Plain,
        send_resolved: true,
    }
}

fn sample_event() -> AlertEvent {
    AlertEvent {
        kind: AlertEventKind::Triggered,
        occurred_at: Utc::now(),
        rule: AlertRuleConfig {
            id: "cpu-hot".to_string(),
            name: "CPU hot".to_string(),
            enabled: true,
            metric: AlertMetric::CpuUsagePercent,
            comparator: AlertComparator::Gt,
            threshold: 90,
            window_minutes: 5,
            severity: AlertSeverity::Critical,
            scope_mode: AlertScopeMode::All,
            node_ids: Vec::new(),
            tags: Vec::new(),
            delivery: vec![AlertChannel::Smtp],
            cooldown_minutes: 30,
            send_resolved: true,
        },
        node_id: "hk-01".to_string(),
        node_label: "Hong Kong".to_string(),
        reading: Some(AlertMetricReading {
            metric: AlertMetric::CpuUsagePercent,
            value: 91,
            threshold: 90,
        }),
    }
}
