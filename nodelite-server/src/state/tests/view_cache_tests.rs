use super::*;

#[tokio::test]
async fn cached_api_json_invalidates_after_visible_status_change() {
    let shared = SharedState::new(Arc::new(sample_config()));
    let session_id = shared
        .register_node(sample_identity(), Some("198.51.100.10".to_string()), None)
        .await;

    let first_nodes = shared.nodes_json_bytes().await.expect("nodes json");
    let first_overview = shared.overview_json_bytes().await.expect("overview json");
    assert_eq!(shared.api_nodes_cache_build_count(), 1);
    assert_eq!(shared.api_overview_cache_build_count(), 1);

    shared.mark_disconnected("hk-01", session_id).await;

    let second_overview = shared
        .overview_json_bytes()
        .await
        .expect("overview json after disconnect");
    assert_eq!(shared.api_nodes_cache_build_count(), 1);
    assert_eq!(shared.api_overview_cache_build_count(), 2);

    let second_nodes = shared
        .nodes_json_bytes()
        .await
        .expect("nodes json after disconnect");
    assert_eq!(shared.api_nodes_cache_build_count(), 2);
    assert_eq!(shared.api_overview_cache_build_count(), 2);

    assert_ne!(first_nodes, second_nodes);
    assert_ne!(first_overview, second_overview);
    assert!(
        std::str::from_utf8(&second_nodes)
            .expect("utf8")
            .contains("\"online\":false")
    );
}

#[tokio::test]
async fn concurrent_api_cache_miss_serializes_once() {
    let shared = SharedState::new(Arc::new(sample_config()));
    shared
        .register_node(sample_identity(), Some("198.51.100.10".to_string()), None)
        .await;

    let mut tasks = Vec::new();
    for _ in 0..10 {
        let shared = shared.clone();
        tasks.push(tokio::spawn(async move {
            shared.nodes_json_bytes().await.expect("nodes json")
        }));
    }

    let mut first = None;
    for task in tasks {
        let body = task.await.expect("task join");
        if let Some(previous) = first.as_ref() {
            assert_eq!(previous, &body);
        } else {
            first = Some(body);
        }
    }

    assert_eq!(shared.api_cache_build_count(), 1);
}

#[tokio::test]
async fn api_overview_and_nodes_caches_build_independently() {
    let shared = SharedState::new(Arc::new(sample_config()));
    shared
        .register_node(sample_identity(), Some("198.51.100.10".to_string()), None)
        .await;

    let first_overview = shared.overview_json_bytes().await.expect("overview json");
    assert_eq!(shared.api_overview_cache_build_count(), 1);
    assert_eq!(
        shared.api_nodes_cache_build_count(),
        0,
        "overview miss must not serialize or populate the nodes body",
    );

    let cached_overview = shared.overview_json_bytes().await.expect("overview json");
    assert_eq!(first_overview, cached_overview);
    assert_eq!(shared.api_overview_cache_build_count(), 1);
    assert_eq!(shared.api_nodes_cache_build_count(), 0);
    let metrics = shared.api_cache_metrics();
    assert_eq!(metrics.overview_hits, 1);
    assert_eq!(metrics.overview_misses, 1);
    assert!(metrics.overview_body_bytes > 0);
    assert_eq!(metrics.nodes_hits, 0);
    assert_eq!(metrics.nodes_misses, 0);
    assert_eq!(metrics.nodes_body_bytes, 0);

    let first_nodes = shared.nodes_json_bytes().await.expect("nodes json");
    assert_eq!(shared.api_overview_cache_build_count(), 1);
    assert_eq!(shared.api_nodes_cache_build_count(), 1);

    let cached_nodes = shared.nodes_json_bytes().await.expect("nodes json");
    assert_eq!(first_nodes, cached_nodes);
    assert_eq!(shared.api_overview_cache_build_count(), 1);
    assert_eq!(shared.api_nodes_cache_build_count(), 1);
    let metrics = shared.api_cache_metrics();
    assert_eq!(metrics.overview_hits, 1);
    assert_eq!(metrics.overview_misses, 1);
    assert_eq!(metrics.nodes_hits, 1);
    assert_eq!(metrics.nodes_misses, 1);
    assert!(metrics.nodes_body_bytes > 0);
    assert!(metrics.overview_body_bytes > 0);
}

#[tokio::test]
async fn snapshot_update_only_invalidates_nodes_view() {
    let shared = SharedState::new(Arc::new(sample_config()));
    let readiness = crate::ServerReadiness::new(true);
    let session_id = shared
        .register_node(sample_identity(), Some("198.51.100.10".to_string()), None)
        .await;

    let _ = shared.overview_json_bytes().await.expect("overview json");
    let _ = shared.nodes_json_bytes().await.expect("nodes json");
    let _ = shared.metrics_text(&readiness).await;
    assert_eq!(shared.api_overview_cache_build_count(), 1);
    assert_eq!(shared.api_nodes_cache_build_count(), 1);
    assert_eq!(shared.metrics_cache_build_count(), 1);

    assert!(
        shared
            .update_snapshot("hk-01", session_id, sample_snapshot(Utc::now()))
            .await
            .is_some()
    );
    let _ = shared.overview_json_bytes().await.expect("overview cached");
    let _ = shared.metrics_text(&readiness).await;
    assert_eq!(shared.api_overview_cache_build_count(), 1);
    assert_eq!(shared.metrics_cache_build_count(), 1);

    let _ = shared.nodes_json_bytes().await.expect("nodes rebuilds");
    assert_eq!(shared.api_nodes_cache_build_count(), 2);

    assert!(shared.update_latency("hk-01", session_id, 42).await);
    let _ = shared.overview_json_bytes().await.expect("overview cached");
    let _ = shared.metrics_text(&readiness).await;
    assert_eq!(shared.api_overview_cache_build_count(), 1);
    assert_eq!(shared.metrics_cache_build_count(), 1);
    let _ = shared
        .nodes_json_bytes()
        .await
        .expect("nodes rebuilds again");
    assert_eq!(shared.api_nodes_cache_build_count(), 3);

    shared.mark_disconnected("hk-01", session_id).await;
    let _ = shared
        .overview_json_bytes()
        .await
        .expect("overview rebuilds");
    let _ = shared.metrics_text(&readiness).await;
    let _ = shared.nodes_json_bytes().await.expect("nodes rebuilds");
    assert_eq!(shared.api_overview_cache_build_count(), 2);
    assert_eq!(shared.metrics_cache_build_count(), 2);
    assert_eq!(shared.api_nodes_cache_build_count(), 4);
}

#[tokio::test]
async fn metrics_cache_reuses_and_invalidates_cleanly() {
    let shared = SharedState::new(Arc::new(sample_config()));
    let readiness = crate::ServerReadiness::new(true);
    let session_id = shared
        .register_node(sample_identity(), Some("198.51.100.10".to_string()), None)
        .await;
    assert!(
        shared
            .update_snapshot("hk-01", session_id, sample_snapshot(Utc::now()))
            .await
            .is_some()
    );

    let mut tasks = Vec::new();
    for _ in 0..10 {
        let shared = shared.clone();
        let readiness = readiness.clone();
        tasks.push(tokio::spawn(async move {
            shared.metrics_text(&readiness).await
        }));
    }

    let mut first = None;
    for task in tasks {
        let body = task.await.expect("task join");
        if let Some(previous) = first.as_ref() {
            assert_eq!(previous, &body);
        } else {
            first = Some(body);
        }
    }
    assert_eq!(shared.metrics_cache_build_count(), 1);

    let cached = shared.metrics_text(&readiness).await;
    assert_eq!(shared.metrics_cache_build_count(), 1);
    assert_eq!(first.expect("first metrics body"), cached);

    shared.mark_disconnected("hk-01", session_id).await;
    let after_disconnect = shared.metrics_text(&readiness).await;
    assert_eq!(shared.metrics_cache_build_count(), 2);
    assert_ne!(cached, after_disconnect);

    readiness.mark_history_available(false);
    let after_readiness = shared.metrics_text(&readiness).await;
    assert_eq!(shared.metrics_cache_build_count(), 3);
    assert_ne!(after_disconnect, after_readiness);
}
