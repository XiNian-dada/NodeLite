use chrono::{Duration as ChronoDuration, Utc};
use serde_json::json;
use tokio::runtime::Runtime;

use super::{AuditEventType, AuditLog, AuditQuery, NewAuditEvent};
use crate::audit::support::{sample_config, unique_temp_dir};

fn persisted_audit_event_count(db_path: &std::path::Path) -> i64 {
    let connection = rusqlite::Connection::open(db_path).expect("audit database should open");
    connection
        .query_row("SELECT COUNT(*) FROM audit_log", [], |row| row.get(0))
        .expect("audit count query should succeed")
}

async fn wait_for_persisted_audit_event_count(db_path: &std::path::Path, expected: i64) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
    loop {
        let actual = persisted_audit_event_count(db_path);
        if actual == expected {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "expected {expected} persisted audit events, found {actual}"
        );
        tokio::task::yield_now().await;
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
}

#[test]
fn audit_log_round_trips_and_filters_events() {
    let runtime = Runtime::new().expect("runtime should build");
    runtime.block_on(async {
        let temp_dir = unique_temp_dir("nodelite-audit-roundtrip");
        std::fs::create_dir_all(&temp_dir).expect("temp dir should exist");
        let db_path = temp_dir.join("audit.sqlite3");
        let audit = AuditLog::new(sample_config(db_path.clone()), 5);
        audit.initialize().await.expect("audit should initialize");

        let mut failure = NewAuditEvent::now(AuditEventType::LoginFailure, "198.51.100.10", false);
        failure.user = Some("viewer".to_string());
        failure.details = json!({"reason":"bad_basic_auth"});
        audit.record(failure).await.expect("failure should persist");

        let mut token = NewAuditEvent::now(AuditEventType::TokenInvalid, "198.51.100.11", false);
        token.node_id = Some("hk-01".to_string());
        token.details = json!({"reason":"expired"});
        audit
            .record(token)
            .await
            .expect("token event should persist");
        audit.shutdown().await;

        let all = audit
            .query(AuditQuery {
                start: None,
                end: None,
                event_type: None,
                success: None,
                limit: 10,
            })
            .await
            .expect("audit query should succeed");
        assert_eq!(all.len(), 2);

        let filtered = audit
            .query(AuditQuery {
                start: None,
                end: None,
                event_type: Some(AuditEventType::LoginFailure),
                success: Some(false),
                limit: 10,
            })
            .await
            .expect("filtered query should succeed");
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].event_type, AuditEventType::LoginFailure);
        assert_eq!(filtered[0].user.as_deref(), Some("viewer"));

        let _ = std::fs::remove_file(&db_path);
        let _ = std::fs::remove_dir_all(&temp_dir);
    });
}

#[test]
fn audit_log_query_combines_optional_filters() {
    let runtime = Runtime::new().expect("runtime should build");
    runtime.block_on(async {
        let temp_dir = unique_temp_dir("nodelite-audit-filter-combo");
        std::fs::create_dir_all(&temp_dir).expect("temp dir should exist");
        let db_path = temp_dir.join("audit.sqlite3");
        let audit = AuditLog::new(sample_config(db_path.clone()), 5);
        audit.initialize().await.expect("audit should initialize");
        let base = Utc::now();

        let stale_failure = NewAuditEvent {
            timestamp: base - ChronoDuration::hours(2),
            event_type: AuditEventType::LoginFailure,
            user: Some("viewer".to_string()),
            node_id: None,
            ip_address: "198.51.100.30".to_string(),
            user_agent: None,
            success: false,
            details: json!({"case":"stale"}),
        };
        let matching_failure = NewAuditEvent {
            timestamp: base,
            event_type: AuditEventType::LoginFailure,
            user: Some("viewer".to_string()),
            node_id: None,
            ip_address: "198.51.100.31".to_string(),
            user_agent: None,
            success: false,
            details: json!({"case":"matching"}),
        };
        let successful_totp = NewAuditEvent {
            timestamp: base,
            event_type: AuditEventType::TotpVerifySuccess,
            user: Some("viewer".to_string()),
            node_id: None,
            ip_address: "198.51.100.32".to_string(),
            user_agent: None,
            success: true,
            details: json!({"case":"success"}),
        };
        audit
            .record(stale_failure)
            .await
            .expect("stale event should enqueue");
        audit
            .record(matching_failure)
            .await
            .expect("matching event should enqueue");
        audit
            .record(successful_totp)
            .await
            .expect("success event should enqueue");

        let events = audit
            .query(AuditQuery {
                start: Some(base - ChronoDuration::minutes(5)),
                end: Some(base + ChronoDuration::minutes(5)),
                event_type: Some(AuditEventType::LoginFailure),
                success: Some(false),
                limit: 10,
            })
            .await
            .expect("combined audit query should succeed");

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].details["case"], "matching");

        audit.shutdown().await;
        let _ = std::fs::remove_file(&db_path);
        let _ = std::fs::remove_dir_all(&temp_dir);
    });
}

#[test]
fn audit_log_prunes_records_older_than_retention_window() {
    let runtime = Runtime::new().expect("runtime should build");
    runtime.block_on(async {
        let temp_dir = unique_temp_dir("nodelite-audit-retention");
        std::fs::create_dir_all(&temp_dir).expect("temp dir should exist");
        let db_path = temp_dir.join("audit.sqlite3");
        let mut config = sample_config(db_path.clone());
        config.retention_days = 1;
        let audit = AuditLog::new(config, 5);
        audit.initialize().await.expect("audit should initialize");

        let old_event = NewAuditEvent {
            timestamp: Utc::now() - ChronoDuration::days(3),
            event_type: AuditEventType::LoginFailure,
            user: None,
            node_id: None,
            ip_address: "203.0.113.10".to_string(),
            user_agent: None,
            success: false,
            details: json!({"reason":"stale"}),
        };
        audit
            .record(old_event)
            .await
            .expect("old event should write");
        audit
            .record(NewAuditEvent::now(
                AuditEventType::TotpVerifyFailure,
                "203.0.113.11",
                false,
            ))
            .await
            .expect("fresh event should write");
        audit.shutdown().await;
        assert_eq!(audit.prune_expired().await.expect("prune should run"), 1);

        let events = audit
            .query(AuditQuery {
                start: None,
                end: None,
                event_type: None,
                success: None,
                limit: 10,
            })
            .await
            .expect("audit query should succeed");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, AuditEventType::TotpVerifyFailure);

        let _ = std::fs::remove_file(&db_path);
        let _ = std::fs::remove_dir_all(&temp_dir);
    });
}

#[test]
fn audit_log_drains_burst_writes_through_writer_task() {
    let runtime = Runtime::new().expect("runtime should build");
    runtime.block_on(async {
        let temp_dir = unique_temp_dir("nodelite-audit-burst");
        std::fs::create_dir_all(&temp_dir).expect("temp dir should exist");
        let db_path = temp_dir.join("audit.sqlite3");
        let audit = AuditLog::new(sample_config(db_path.clone()), 5);
        audit.initialize().await.expect("audit should initialize");

        for index in 0..1000 {
            let mut event = NewAuditEvent::now(
                AuditEventType::RateLimitExceeded,
                format!("198.51.100.{}", index % 255),
                false,
            );
            event.details = json!({"attempt": index});
            audit
                .record(event)
                .await
                .expect("burst audit event should enqueue");
        }

        audit.shutdown().await;
        let events = audit
            .query(AuditQuery {
                start: None,
                end: None,
                event_type: Some(AuditEventType::RateLimitExceeded),
                success: Some(false),
                limit: 1000,
            })
            .await
            .expect("audit query should succeed");

        assert_eq!(events.len(), 1000);

        let _ = std::fs::remove_file(&db_path);
        let _ = std::fs::remove_dir_all(&temp_dir);
    });
}

#[tokio::test(start_paused = true)]
async fn audit_writer_flushes_when_configured_batch_max_is_reached() {
    let temp_dir = unique_temp_dir("nodelite-audit-configured-batch");
    std::fs::create_dir_all(&temp_dir).expect("temp dir should exist");
    let db_path = temp_dir.join("audit.sqlite3");
    let mut config = sample_config(db_path.clone());
    config.writer_batch_max = 2;
    config.writer_flush_interval_ms = 60_000;
    let audit = AuditLog::new(config, 5);
    audit.initialize().await.expect("audit should initialize");
    tokio::task::yield_now().await;

    let paused_at = tokio::time::Instant::now();
    audit
        .record(NewAuditEvent::now(
            AuditEventType::RateLimitExceeded,
            "198.51.100.40",
            false,
        ))
        .await
        .expect("first audit event should enqueue");
    tokio::task::yield_now().await;
    assert_eq!(persisted_audit_event_count(&db_path), 0);

    audit
        .record(NewAuditEvent::now(
            AuditEventType::RateLimitExceeded,
            "198.51.100.41",
            false,
        ))
        .await
        .expect("second audit event should enqueue");
    wait_for_persisted_audit_event_count(&db_path, 2).await;
    assert_eq!(tokio::time::Instant::now(), paused_at);

    audit.shutdown().await;
    let _ = std::fs::remove_file(&db_path);
    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[tokio::test(start_paused = true)]
async fn audit_writer_flushes_at_configured_interval() {
    let temp_dir = unique_temp_dir("nodelite-audit-configured-interval");
    std::fs::create_dir_all(&temp_dir).expect("temp dir should exist");
    let db_path = temp_dir.join("audit.sqlite3");
    let mut config = sample_config(db_path.clone());
    config.writer_batch_max = 128;
    config.writer_flush_interval_ms = 10;
    let audit = AuditLog::new(config, 5);
    audit.initialize().await.expect("audit should initialize");
    tokio::task::yield_now().await;

    audit
        .record(NewAuditEvent::now(
            AuditEventType::RateLimitExceeded,
            "198.51.100.42",
            false,
        ))
        .await
        .expect("audit event should enqueue");
    tokio::task::yield_now().await;
    assert_eq!(persisted_audit_event_count(&db_path), 0);

    let before_advance = tokio::time::Instant::now();
    tokio::time::advance(std::time::Duration::from_millis(10)).await;
    wait_for_persisted_audit_event_count(&db_path, 1).await;
    assert_eq!(
        tokio::time::Instant::now(),
        before_advance + std::time::Duration::from_millis(10)
    );

    audit.shutdown().await;
    let _ = std::fs::remove_file(&db_path);
    let _ = std::fs::remove_dir_all(&temp_dir);
}
