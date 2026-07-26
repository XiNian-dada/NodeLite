//! Server 默认值、示例配置与资源并发测试。

use std::path::PathBuf;

use super::super::{
    AlertChannel, AlertMetric, AlertSeverity, DEFAULT_ALERT_CPU_WINDOW_MINUTES,
    DEFAULT_ALERT_INSPECTION_CPU_WARN_PERCENT, DEFAULT_ALERT_INSPECTION_LATENCY_WARN_MS,
    DEFAULT_ALERT_INSPECTION_LOCAL_TIME, DEFAULT_ALERT_INSPECTION_MEMORY_WARN_PERCENT,
    DEFAULT_ALERT_MEMORY_WINDOW_MINUTES, DEFAULT_ALERT_OFFLINE_THRESHOLD_MINUTES,
    DEFAULT_ALERT_RTT_WINDOW_MINUTES, DEFAULT_AUDIT_RETENTION_DAYS, DEFAULT_AUDIT_WRITER_BATCH_MAX,
    DEFAULT_AUDIT_WRITER_FLUSH_INTERVAL_MS, DEFAULT_GEOIP_UPDATE_INTERVAL_DAYS,
    DEFAULT_HISTORY_QUERY_CONCURRENCY, DEFAULT_HISTORY_READ_CACHE_KIB,
    DEFAULT_HISTORY_WRITER_BATCH_MAX, DEFAULT_HISTORY_WRITER_FLUSH_INTERVAL_MS,
    DEFAULT_MAX_MESSAGE_BYTES, DEFAULT_TOKEN_VERIFY_MAX_PARALLELISM, DEFAULT_WS_AUTH_BLOCK_SECS,
    DEFAULT_WS_AUTH_FAIL_MAX_ATTEMPTS, DEFAULT_WS_AUTH_FAIL_WINDOW_SECS,
    DEFAULT_WS_MAX_CONNECTIONS_PER_IP, DEFAULT_WS_MAX_TOTAL_CONNECTIONS, GeoIpEdition,
    GeoIpProvider, MAX_HISTORY_QUERY_CONCURRENCY, MAX_HISTORY_READ_CACHE_KIB,
    MAX_WRITER_BATCH_SIZE, MIN_HISTORY_QUERY_CONCURRENCY, MIN_HISTORY_READ_CACHE_KIB,
    MIN_WRITER_FLUSH_INTERVAL_MS, parse_server_config,
};

#[test]
fn server_example_documents_install_section() {
    let example = include_str!("../../../../config/server.example.toml");

    assert!(example.contains("[install]"));
    assert!(example.contains("agent_release_base_url"));
    assert!(example.contains("agent_release_sha256_x86_64"));
    assert!(example.contains("agent_release_sha256_aarch64"));
}

#[test]
fn server_example_documents_metrics_section() {
    let example = include_str!("../../../../config/server.example.toml");

    assert!(example.contains("[metrics]"));
    assert!(example.contains("export_node_resource_metrics"));
    assert!(example.contains("export_node_disk_metrics"));
}

#[test]
fn server_example_documents_token_verify_parallelism() {
    let example = include_str!("../../../../config/server.example.toml");

    assert!(example.contains("token_verify_max_parallelism = 4"));
    assert!(example.contains("每个任务约需 19 MiB"));
}

#[test]
fn server_example_documents_history_query_limits() {
    let example = include_str!("../../../../config/server.example.toml");

    assert!(example.contains("history_query_concurrency"));
    assert!(example.contains("history_read_cache_kib"));
}

#[test]
fn server_example_documents_writer_scheduling() {
    let example = include_str!("../../../../config/server.example.toml");

    for expected in [
        "history_writer_batch_max = 128",
        "history_writer_flush_interval_ms = 100",
        "writer_batch_max = 128",
        "writer_flush_interval_ms = 100",
    ] {
        assert!(example.lines().any(|line| line == expected));
    }
    assert_eq!(example.matches("允许 1-4096").count(), 2);
}

#[test]
fn parses_server_config_with_defaults() {
    let config = parse_server_config(
        r#"
        [server]
        listen = "127.0.0.1:8080"
        public_base_url = "http://127.0.0.1:8080"
        "#,
    )
    .expect("server config should parse");

    assert_eq!(config.listen.to_string(), "127.0.0.1:8080");
    assert!(!config.insecure_allow_http);
    assert_eq!(config.readonly_auth, None);
    assert!(config.trusted_proxies.is_empty());
    assert_eq!(config.max_message_bytes, DEFAULT_MAX_MESSAGE_BYTES);
    assert_eq!(
        config.history_query_concurrency,
        DEFAULT_HISTORY_QUERY_CONCURRENCY
    );
    assert_eq!(
        config.history_read_cache_kib,
        DEFAULT_HISTORY_READ_CACHE_KIB
    );
    assert_eq!(
        config.history_writer_batch_max,
        DEFAULT_HISTORY_WRITER_BATCH_MAX
    );
    assert_eq!(
        config.history_writer_flush_interval_ms,
        DEFAULT_HISTORY_WRITER_FLUSH_INTERVAL_MS
    );
    assert_eq!(
        config.ws.max_total_connections,
        DEFAULT_WS_MAX_TOTAL_CONNECTIONS
    );
    assert_eq!(
        config.ws.max_connections_per_ip,
        DEFAULT_WS_MAX_CONNECTIONS_PER_IP
    );
    assert_eq!(
        config.ws.auth_fail_window_secs,
        DEFAULT_WS_AUTH_FAIL_WINDOW_SECS
    );
    assert_eq!(
        config.ws.auth_fail_max_attempts,
        DEFAULT_WS_AUTH_FAIL_MAX_ATTEMPTS
    );
    assert_eq!(config.ws.auth_block_secs, DEFAULT_WS_AUTH_BLOCK_SECS);
    assert_eq!(
        config.token_verify_max_parallelism,
        DEFAULT_TOKEN_VERIFY_MAX_PARALLELISM
    );
    assert!(!config.metrics.export_node_resource_metrics);
    assert!(!config.metrics.export_node_disk_metrics);
    assert_eq!(
        config.node_registry_path,
        PathBuf::from("./config/server.json")
    );
    assert_eq!(
        config.ignored_filesystems,
        vec!["devtmpfs", "overlay", "tmpfs"]
    );
    assert!(config.audit.enabled);
    assert_eq!(config.audit.db_path, PathBuf::from("./data/audit.sqlite3"));
    assert_eq!(config.audit.retention_days, DEFAULT_AUDIT_RETENTION_DAYS);
    assert_eq!(
        config.audit.writer_batch_max,
        DEFAULT_AUDIT_WRITER_BATCH_MAX
    );
    assert_eq!(
        config.audit.writer_flush_interval_ms,
        DEFAULT_AUDIT_WRITER_FLUSH_INTERVAL_MS
    );
    assert!(config.audit.log_successful_auth);
    assert!(config.audit.log_failed_auth);
    assert!(config.audit.log_token_events);
    assert!(config.audit.log_rate_limit);
    assert!(config.geoip.enabled);
    assert_eq!(config.geoip.provider, GeoIpProvider::Ipwhois);
    assert_eq!(config.geoip.edition, GeoIpEdition::CountryLite);
    assert_eq!(
        config.geoip.database_path,
        PathBuf::from("./data/geoip/dbip.mmdb")
    );
    assert!(!config.geoip.auto_update);
    assert_eq!(
        config.geoip.update_interval_days,
        DEFAULT_GEOIP_UPDATE_INTERVAL_DAYS
    );
    assert!(!config.alerting.enabled);
    assert_eq!(config.alerting.rules.len(), 4);
    assert_eq!(config.alerting.rules[0].id, "node-offline");
    assert_eq!(config.alerting.rules[0].metric, AlertMetric::OfflineMinutes);
    assert_eq!(
        config.alerting.rules[0].threshold,
        DEFAULT_ALERT_OFFLINE_THRESHOLD_MINUTES
    );
    assert_eq!(config.alerting.rules[0].window_minutes, 1);
    assert_eq!(config.alerting.rules[0].severity, AlertSeverity::Critical);
    assert_eq!(config.alerting.rules[1].id, "cpu-avg-hot");
    assert_eq!(
        config.alerting.rules[1].threshold,
        DEFAULT_ALERT_INSPECTION_CPU_WARN_PERCENT
    );
    assert_eq!(
        config.alerting.rules[1].window_minutes,
        DEFAULT_ALERT_CPU_WINDOW_MINUTES
    );
    assert_eq!(config.alerting.rules[2].id, "memory-avg-hot");
    assert_eq!(
        config.alerting.rules[2].threshold,
        DEFAULT_ALERT_INSPECTION_MEMORY_WARN_PERCENT
    );
    assert_eq!(
        config.alerting.rules[2].window_minutes,
        DEFAULT_ALERT_MEMORY_WINDOW_MINUTES
    );
    assert_eq!(config.alerting.rules[3].id, "rtt-avg-high");
    assert_eq!(
        config.alerting.rules[3].threshold,
        DEFAULT_ALERT_INSPECTION_LATENCY_WARN_MS
    );
    assert_eq!(
        config.alerting.rules[3].window_minutes,
        DEFAULT_ALERT_RTT_WINDOW_MINUTES
    );
    assert!(
        config
            .alerting
            .rules
            .iter()
            .all(|rule| rule.delivery == vec![AlertChannel::Smtp, AlertChannel::Webhook])
    );
    assert_eq!(
        config.alerting.inspection.local_time,
        DEFAULT_ALERT_INSPECTION_LOCAL_TIME
    );
}

#[test]
fn parses_history_query_resource_overrides() {
    let config = parse_server_config(
        r#"
        [server]
        listen = "127.0.0.1:8080"
        public_base_url = "https://monitor.example.com"
        history_query_concurrency = 8
        history_read_cache_kib = 1024
        "#,
    )
    .expect("history query resource overrides should parse");

    assert_eq!(config.history_query_concurrency, 8);
    assert_eq!(config.history_read_cache_kib, 1024);
}

#[test]
fn parses_writer_scheduling_overrides() {
    let config = parse_server_config(
        r#"
        [server]
        listen = "127.0.0.1:8080"
        public_base_url = "https://monitor.example.com"
        history_writer_batch_max = 64
        history_writer_flush_interval_ms = 25

        [audit]
        writer_batch_max = 32
        writer_flush_interval_ms = 50
        "#,
    )
    .expect("writer scheduling overrides should parse");

    assert_eq!(config.history_writer_batch_max, 64);
    assert_eq!(config.history_writer_flush_interval_ms, 25);
    assert_eq!(config.audit.writer_batch_max, 32);
    assert_eq!(config.audit.writer_flush_interval_ms, 50);
}

#[test]
fn accepts_writer_batch_size_upper_bound() {
    let config = parse_server_config(&format!(
        r#"
        [server]
        listen = "127.0.0.1:8080"
        public_base_url = "https://monitor.example.com"
        history_writer_batch_max = {MAX_WRITER_BATCH_SIZE}

        [audit]
        writer_batch_max = {MAX_WRITER_BATCH_SIZE}
        "#,
    ))
    .expect("writer batch upper bound should parse");

    assert_eq!(config.history_writer_batch_max, MAX_WRITER_BATCH_SIZE);
    assert_eq!(config.audit.writer_batch_max, MAX_WRITER_BATCH_SIZE);
}

#[test]
fn rejects_invalid_writer_scheduling_values() {
    assert_eq!(MIN_WRITER_FLUSH_INTERVAL_MS, 10);
    assert_eq!(MAX_WRITER_BATCH_SIZE, 4096);
    for (section, key, value, expected) in [
        (
            "server",
            "history_writer_batch_max",
            0,
            "server.history_writer_batch_max",
        ),
        ("audit", "writer_batch_max", 0, "audit.writer_batch_max"),
        (
            "server",
            "history_writer_batch_max",
            MAX_WRITER_BATCH_SIZE.saturating_add(1),
            "server.history_writer_batch_max",
        ),
        (
            "audit",
            "writer_batch_max",
            MAX_WRITER_BATCH_SIZE.saturating_add(1),
            "audit.writer_batch_max",
        ),
    ] {
        let setting = if section == "server" {
            format!("{key} = {value}")
        } else {
            format!("[audit]\n{key} = {value}")
        };
        let input = format!(
            r#"
            [server]
            listen = "127.0.0.1:8080"
            public_base_url = "https://monitor.example.com"
            {setting}
            "#,
        );
        let error = parse_server_config(&input).expect_err("invalid writer batch should fail");
        assert!(error.to_string().contains(expected));
    }

    for (section, key, value, expected) in [
        (
            "server",
            "history_writer_flush_interval_ms",
            0,
            "server.history_writer_flush_interval_ms",
        ),
        (
            "audit",
            "writer_flush_interval_ms",
            0,
            "audit.writer_flush_interval_ms",
        ),
        (
            "server",
            "history_writer_flush_interval_ms",
            MIN_WRITER_FLUSH_INTERVAL_MS - 1,
            "server.history_writer_flush_interval_ms",
        ),
        (
            "audit",
            "writer_flush_interval_ms",
            MIN_WRITER_FLUSH_INTERVAL_MS - 1,
            "audit.writer_flush_interval_ms",
        ),
    ] {
        let setting = if section == "server" {
            format!("{key} = {value}")
        } else {
            format!("[audit]\n{key} = {value}")
        };
        let input = format!(
            r#"
            [server]
            listen = "127.0.0.1:8080"
            public_base_url = "https://monitor.example.com"
            {setting}
            "#,
        );
        let error = parse_server_config(&input).expect_err("invalid writer interval should fail");
        assert!(error.to_string().contains(expected));
    }
}

#[test]
fn rejects_history_query_resource_limits_outside_safe_ranges() {
    assert_eq!(MIN_HISTORY_QUERY_CONCURRENCY, 1);
    assert_eq!(MAX_HISTORY_QUERY_CONCURRENCY, 8);
    assert_eq!(MIN_HISTORY_READ_CACHE_KIB, 64);
    assert_eq!(MAX_HISTORY_READ_CACHE_KIB, 1024);

    for (key, value, expected) in [
        (
            "history_query_concurrency",
            MIN_HISTORY_QUERY_CONCURRENCY.saturating_sub(1).to_string(),
            "server.history_query_concurrency",
        ),
        (
            "history_query_concurrency",
            MAX_HISTORY_QUERY_CONCURRENCY.saturating_add(1).to_string(),
            "server.history_query_concurrency",
        ),
        (
            "history_read_cache_kib",
            MIN_HISTORY_READ_CACHE_KIB.saturating_sub(1).to_string(),
            "server.history_read_cache_kib",
        ),
        (
            "history_read_cache_kib",
            MAX_HISTORY_READ_CACHE_KIB.saturating_add(1).to_string(),
            "server.history_read_cache_kib",
        ),
    ] {
        let input = format!(
            r#"
            [server]
            listen = "127.0.0.1:8080"
            public_base_url = "https://monitor.example.com"
            {key} = {value}
            "#,
        );
        let error = parse_server_config(&input).expect_err("unsafe history limit should fail");
        assert!(error.to_string().contains(expected));
    }
}

#[test]
fn parses_token_verify_parallelism_override() {
    let config = parse_server_config(
        r#"
        [server]
        listen = "127.0.0.1:8080"
        public_base_url = "https://monitor.example.com"
        token_verify_max_parallelism = 2
        "#,
    )
    .expect("token verify parallelism should parse");

    assert_eq!(config.token_verify_max_parallelism, 2);
}

#[test]
fn rejects_token_verify_parallelism_outside_safe_range() {
    for invalid in [0, 9] {
        let input = format!(
            r#"
            [server]
            listen = "127.0.0.1:8080"
            public_base_url = "https://monitor.example.com"
            token_verify_max_parallelism = {invalid}
            "#
        );
        let error = parse_server_config(&input)
            .expect_err("out-of-range token verify parallelism should fail");

        assert!(
            error
                .to_string()
                .contains("server.token_verify_max_parallelism must be between 1 and 8")
        );
    }
}
