//! Log budget, sanitization, and concurrent writer regressions.

use chrono::Utc;
use nodelite_proto::{AgentLogsConfig, NoticeLevel};
use tokio::task::JoinSet;

use super::{AgentLogEntry, AgentLogStore, MAX_BATCH_ENTRIES, MAX_LOGS_PER_NODE};

const MAX_LOG_ENTRIES_TOTAL: usize = 10_000;

#[tokio::test]
async fn record_entries_caps_per_node_and_surfaces_drops() {
    let store = AgentLogStore::new();
    let total = MAX_LOGS_PER_NODE + 10;
    let entries = (0..total)
        .map(|index| AgentLogEntry {
            occurred_at: "invalid".to_string(),
            level: NoticeLevel::Info,
            message: format!("entry-{index}"),
        })
        .collect();

    let result = store.record_entries("hk-01", entries).await;
    // #89: 接受恰好 MAX_BATCH_ENTRIES 条 (sanitize 都通过, 因为 message 都非空),
    // 多出来的部分由 dropped_batch_cap 报出 —— 不再像旧版那样静默丢失。
    assert_eq!(result.accepted, MAX_BATCH_ENTRIES);
    assert_eq!(result.dropped_batch_cap, total - MAX_BATCH_ENTRIES);
    assert_eq!(result.dropped_sanitize, 0);
    assert_eq!(result.evicted_global_budget, 0);
    assert_eq!(result.total_dropped(), total - MAX_BATCH_ENTRIES);

    let logs = store.list("hk-01", MAX_LOGS_PER_NODE).await;
    assert_eq!(logs.len(), MAX_BATCH_ENTRIES);
    assert!(logs.iter().all(|entry| !entry.message.is_empty()));
    assert!(
        logs.iter()
            .all(|entry| chrono::DateTime::parse_from_rfc3339(&entry.occurred_at).is_ok())
    );
}

#[tokio::test]
async fn record_entries_counts_sanitize_drops() {
    // sanitize_entry 拒掉空消息与纯空白消息,这些应当计入 dropped_sanitize
    // 而不是 accepted。
    let store = AgentLogStore::new();
    let entries = vec![
        AgentLogEntry {
            occurred_at: Utc::now().to_rfc3339(),
            level: NoticeLevel::Info,
            message: "  ".to_string(), // sanitize_entry returns None
        },
        AgentLogEntry {
            occurred_at: Utc::now().to_rfc3339(),
            level: NoticeLevel::Info,
            message: "real entry".to_string(),
        },
    ];

    let result = store.record_entries("hk-01", entries).await;
    assert_eq!(result.accepted, 1);
    assert_eq!(result.dropped_batch_cap, 0);
    assert_eq!(result.dropped_sanitize, 1);
    assert_eq!(result.evicted_global_budget, 0);
}

#[tokio::test]
async fn forget_missing_prunes_retired_node_buffers() {
    let store = AgentLogStore::new();
    let entry = AgentLogEntry {
        occurred_at: Utc::now().to_rfc3339(),
        level: NoticeLevel::Warn,
        message: "reconnecting".to_string(),
    };
    store.record_entries("hk-01", vec![entry.clone()]).await;
    store.record_entries("jp-01", vec![entry]).await;

    let removed = store.forget_missing(&["jp-01".to_string()]).await;
    assert_eq!(removed, 1);
    assert!(store.list("hk-01", 10).await.is_empty());
    assert_eq!(store.list("jp-01", 10).await.len(), 1);

    let stats = store.stats().await;
    assert_eq!(stats.nodes, 1);
    assert_eq!(stats.entries, 1);
}

#[tokio::test]
async fn global_entry_budget_evicts_oldest_logs_across_nodes() {
    let store = AgentLogStore::new();
    let mut evicted = 0;

    for node_index in 0..=MAX_LOG_ENTRIES_TOTAL / MAX_LOGS_PER_NODE {
        let node_id = format!("node-{node_index:03}");
        for chunk_start in (0..MAX_LOGS_PER_NODE).step_by(MAX_BATCH_ENTRIES) {
            let chunk_len = MAX_BATCH_ENTRIES.min(MAX_LOGS_PER_NODE - chunk_start);
            let entries = (chunk_start..chunk_start + chunk_len)
                .map(|entry_index| test_entry(format!("{node_id}-entry-{entry_index:03}")))
                .collect();
            let result = store.record_entries(&node_id, entries).await;
            assert_eq!(result.dropped_batch_cap, 0);
            assert_eq!(result.dropped_sanitize, 0);
            evicted += result.evicted_global_budget;
        }
    }

    let stats = store.stats().await;
    assert_eq!(stats.entries, MAX_LOG_ENTRIES_TOTAL);
    assert!(stats.estimated_bytes <= stats.max_estimated_bytes);
    assert_eq!(evicted, MAX_LOGS_PER_NODE);
    assert!(store.list("node-000", 1).await.is_empty());

    let newest_node = format!("node-{:03}", MAX_LOG_ENTRIES_TOTAL / MAX_LOGS_PER_NODE);
    let recent = store.list(&newest_node, 3).await;
    assert_eq!(recent.len(), 3);
    assert_eq!(
        recent
            .first()
            .expect("recent log should include first visible entry")
            .message,
        format!("{newest_node}-entry-197")
    );
    assert_eq!(
        recent
            .last()
            .expect("recent log should include newest entry")
            .message,
        format!("{newest_node}-entry-199")
    );
}

#[tokio::test]
async fn global_byte_budget_evicts_oldest_logs_and_reports_stats() {
    let store = AgentLogStore::with_limits(AgentLogsConfig {
        max_entries: 10_000,
        max_estimated_bytes: 64 * 1024,
    });
    let mut evicted = 0;
    for _ in 0..8 {
        let result = store
            .record_entries(
                "heavy-node",
                (0..64).map(|_| test_entry("x".repeat(512))).collect(),
            )
            .await;
        evicted += result.evicted_global_budget;
    }
    let stats = store.stats().await;
    assert!(evicted > 0);
    assert_eq!(stats.evicted_global_budget_total, evicted as u64);
    assert!(stats.entries < MAX_LOGS_PER_NODE);
    assert!(stats.estimated_bytes <= 64 * 1024);
    assert_eq!(stats.max_estimated_bytes, 64 * 1024);
    assert_eq!(stats.max_entries, 10_000);
}

#[tokio::test]
async fn concurrent_record_entries_preserves_batches_across_many_nodes() {
    let store = AgentLogStore::new();
    let mut tasks = JoinSet::new();
    let node_count = 128;

    for node_index in 0..node_count {
        let store = store.clone();
        tasks.spawn(async move {
            let node_id = format!("node-{node_index:03}");
            let entries = (0..MAX_BATCH_ENTRIES)
                .map(|entry_index| test_entry(format!("{node_id}-entry-{entry_index:03}")))
                .collect();
            let result = store.record_entries(&node_id, entries).await;
            (node_id, result)
        });
    }

    while let Some(joined) = tasks.join_next().await {
        let (node_id, result) = joined.expect("concurrent log write task should join");
        assert_eq!(result.accepted, MAX_BATCH_ENTRIES, "{node_id}");
        assert_eq!(result.dropped_batch_cap, 0, "{node_id}");
        assert_eq!(result.dropped_sanitize, 0, "{node_id}");
        assert_eq!(result.evicted_global_budget, 0, "{node_id}");
    }

    let stats = store.stats().await;
    assert_eq!(stats.nodes, node_count);
    assert_eq!(stats.entries, node_count * MAX_BATCH_ENTRIES);

    let sample = store.list("node-064", MAX_BATCH_ENTRIES).await;
    assert_eq!(sample.len(), MAX_BATCH_ENTRIES);
    assert_eq!(
        sample
            .first()
            .expect("sample should contain oldest kept entry")
            .message,
        "node-064-entry-000"
    );
    assert_eq!(
        sample
            .last()
            .expect("sample should contain newest kept entry")
            .message,
        format!("node-064-entry-{:03}", MAX_BATCH_ENTRIES - 1)
    );
}

fn test_entry(message: String) -> AgentLogEntry {
    AgentLogEntry {
        occurred_at: Utc::now().to_rfc3339(),
        level: NoticeLevel::Info,
        message,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_low_memory_writes_never_leave_a_budget_overrun() {
    let store = AgentLogStore::with_limits(AgentLogsConfig {
        max_entries: 128,
        max_estimated_bytes: 2 * 1024 * 1024,
    });
    let mut tasks = JoinSet::new();
    for node in 0..64 {
        let store = store.clone();
        tasks.spawn(async move {
            for _ in 0..4 {
                store
                    .record_entries(
                        &format!("node-{node}"),
                        (0..64).map(|_| test_entry("x".repeat(512))).collect(),
                    )
                    .await;
                let stats = store.stats().await;
                assert!(stats.entries <= stats.max_entries);
                assert!(stats.estimated_bytes <= stats.max_estimated_bytes);
            }
        });
    }
    while let Some(result) = tasks.join_next().await {
        result.expect("concurrent writer");
    }
    let stats = store.stats().await;
    assert_eq!(stats.entries, 128);
    assert!(stats.evicted_global_budget_total > 0);
    store.forget_missing(&[]).await;
    assert_eq!(store.stats().await.entries, 0);
    assert_eq!(store.stats().await.estimated_bytes, 0);
    assert_eq!(store.inner.lock().await.buffers.capacity(), 0);
}

#[tokio::test]
async fn default_store_is_initialized_and_cancellation_cannot_leak_accounting() {
    let store = AgentLogStore::default();
    let guard = store.inner.lock().await;
    let writer = store.clone();
    let task = tokio::spawn(async move {
        writer
            .record_entries("node", vec![test_entry("hello".into())])
            .await
    });
    tokio::task::yield_now().await;
    task.abort();
    assert!(task.await.expect_err("writer cancelled").is_cancelled());
    drop(guard);
    assert_eq!(store.stats().await.entries, 0);
    store
        .record_entries("node", vec![test_entry("hello".into())])
        .await;
    assert_eq!(store.list("node", 10).await.len(), 1);
}

#[tokio::test]
async fn node_retention_and_input_drops_are_counted() {
    let store = AgentLogStore::new();
    for _ in 0..4 {
        store
            .record_entries(
                "node",
                (0..64).map(|_| test_entry("hello".into())).collect(),
            )
            .await;
    }
    assert_eq!(store.stats().await.evicted_per_node_total, 56);
    let result = store
        .record_entries(&"x".repeat(129), vec![test_entry("hello".into())])
        .await;
    assert_eq!(result.dropped_sanitize, 1);
    assert_eq!(store.stats().await.dropped_total, 1);
}
