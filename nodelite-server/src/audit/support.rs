use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

pub(super) fn unique_temp_dir(prefix: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be monotonic enough")
        .as_nanos();
    std::env::temp_dir().join(format!("{prefix}-{unique}"))
}

pub(super) fn sample_config(db_path: PathBuf) -> nodelite_proto::AuditConfig {
    nodelite_proto::AuditConfig {
        enabled: true,
        db_path,
        retention_days: 90,
        writer_batch_max: nodelite_proto::DEFAULT_AUDIT_WRITER_BATCH_MAX,
        writer_flush_interval_ms: nodelite_proto::DEFAULT_AUDIT_WRITER_FLUSH_INTERVAL_MS,
        log_successful_auth: true,
        log_failed_auth: true,
        log_token_events: true,
        log_rate_limit: true,
    }
}
