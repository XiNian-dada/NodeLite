//! 浏览器增量广播的规模边界测试。

use std::collections::HashMap;

use futures::future::join_all;
use tokio::sync::broadcast::error::{RecvError, TryRecvError};

use axum::extract::ws::Utf8Bytes;

use super::super::browser_broadcast::{
    BROWSER_INCREMENTAL_CHANNEL_CAPACITY, BrowserIncrementalUpdate, run_tick,
};
use super::*;

async fn register_nodes(shared: &SharedState, count: usize) {
    for index in 0..count {
        let mut identity = sample_identity();
        identity.node_id = format!("node-{index:04}");
        shared.register_node(identity, None, None, None).await;
    }
}

#[tokio::test]
async fn one_tick_uses_one_channel_slot_regardless_of_node_count() {
    let shared = SharedState::new(Arc::new(sample_config()));
    register_nodes(&shared, 300).await;
    let mut subscriber = shared.subscribe_browser_incremental();
    let mut last_nodes = HashMap::new();
    let mut dirty = true;

    run_tick(&shared, &mut last_nodes, &mut dirty).await;

    let update = subscriber.try_recv().expect("batched update");
    assert_eq!(update.frames.len(), 301, "300 upserts plus one overview");
    assert!(matches!(subscriber.try_recv(), Err(TryRecvError::Empty)));
    assert!(!dirty);
}

#[tokio::test]
async fn tick_without_subscribers_does_no_work_and_forgets_the_baseline() {
    let shared = SharedState::new(Arc::new(sample_config()));
    register_nodes(&shared, 2).await;
    let mut subscriber = shared.subscribe_browser_incremental();
    let mut last_nodes = HashMap::new();
    let mut dirty = true;
    run_tick(&shared, &mut last_nodes, &mut dirty).await;
    assert_eq!(last_nodes.len(), 2);
    subscriber.try_recv().expect("first batch");
    drop(subscriber);

    dirty = true;
    run_tick(&shared, &mut last_nodes, &mut dirty).await;
    assert!(last_nodes.is_empty());
    assert!(!dirty);

    let mut subscriber = shared.subscribe_browser_incremental();
    dirty = true;
    run_tick(&shared, &mut last_nodes, &mut dirty).await;
    let update = subscriber.try_recv().expect("rebuilt batch");
    assert_eq!(
        update.frames.len(),
        3,
        "every node is resent after the reset"
    );
}

#[tokio::test]
async fn concurrent_full_syncs_share_one_build() {
    let shared = SharedState::new(Arc::new(sample_config()));
    register_nodes(&shared, 10).await;

    let results = join_all((0..64).map(|_| shared.browser_initial_state_frame())).await;

    assert_eq!(shared.browser_initial_state_build_count(), 1);
    let (revision, frame) = results[0].as_ref().expect("initial state");
    for result in &results {
        let (other_revision, other_frame) = result.as_ref().expect("initial state");
        assert_eq!(other_revision, revision);
        assert_eq!(other_frame, frame);
    }
}

#[tokio::test]
async fn lagged_sessions_resync_with_one_build_per_tick() {
    let shared = SharedState::new(Arc::new(sample_config()));
    register_nodes(&shared, 2).await;
    let mut sessions: Vec<_> = (0..32)
        .map(|_| shared.subscribe_browser_incremental())
        .collect();
    let mut last_nodes = HashMap::new();
    for _ in 0..32 {
        let mut dirty = true;
        run_tick(&shared, &mut last_nodes, &mut dirty).await;
    }

    for session in &mut sessions {
        assert!(matches!(session.recv().await, Err(RecvError::Lagged(_))));
        shared
            .browser_initial_state_frame()
            .await
            .expect("resync frame");
    }

    assert_eq!(shared.browser_initial_state_build_count(), 1);
}

#[tokio::test]
async fn published_batch_invalidates_the_shared_full_state() {
    let shared = SharedState::new(Arc::new(sample_config()));
    register_nodes(&shared, 1).await;
    let _subscriber = shared.subscribe_browser_incremental();
    let mut last_nodes = HashMap::new();
    let (first, _) = shared
        .browser_initial_state_frame()
        .await
        .expect("first frame");

    let mut late = sample_identity();
    late.node_id = "added-node".to_string();
    shared.register_node(late, None, None, None).await;
    let mut dirty = true;
    run_tick(&shared, &mut last_nodes, &mut dirty).await;

    // 这一批已发布,之后才订阅的会话收不到它,不能再拿旧快照。
    let (second, frame) = shared
        .browser_initial_state_frame()
        .await
        .expect("second frame");
    assert!(second > first);
    assert!(frame.as_str().contains("added-node"));
    assert_eq!(shared.browser_initial_state_build_count(), 2);
}

#[tokio::test]
async fn unpublished_change_reaches_a_session_on_the_shared_full_state() {
    let shared = SharedState::new(Arc::new(sample_config()));
    register_nodes(&shared, 1).await;
    let mut subscriber = shared.subscribe_browser_incremental();
    let mut last_nodes = HashMap::new();
    let mut dirty = true;
    run_tick(&shared, &mut last_nodes, &mut dirty).await;
    subscriber.try_recv().expect("first batch");
    shared
        .browser_initial_state_frame()
        .await
        .expect("cached frame");

    let mut late = sample_identity();
    late.node_id = "added-node".to_string();
    shared.register_node(late, None, None, None).await;

    let (baseline, frame) = shared
        .browser_initial_state_frame()
        .await
        .expect("reused frame");
    assert_eq!(shared.browser_initial_state_build_count(), 1);
    assert!(!frame.as_str().contains("added-node"));

    dirty = true;
    run_tick(&shared, &mut last_nodes, &mut dirty).await;
    let update = subscriber.try_recv().expect("next batch");
    assert!(update.revision > baseline);
    assert!(
        update
            .frames
            .iter()
            .any(|f| f.as_str().contains("added-node"))
    );
}

#[tokio::test]
async fn idle_tick_drops_the_shared_full_state() {
    let shared = SharedState::new(Arc::new(sample_config()));
    register_nodes(&shared, 1).await;
    shared
        .browser_initial_state_frame()
        .await
        .expect("cached frame");
    let mut last_nodes = HashMap::new();
    let mut dirty = false;

    run_tick(&shared, &mut last_nodes, &mut dirty).await;
    shared
        .browser_initial_state_frame()
        .await
        .expect("rebuilt frame");

    assert_eq!(shared.browser_initial_state_build_count(), 2);
}

fn retained_bytes(batches: &[std::sync::Weak<[Utf8Bytes]>]) -> usize {
    batches
        .iter()
        .filter_map(std::sync::Weak::upgrade)
        .map(|frames| frames.iter().map(|frame| frame.len()).sum::<usize>())
        .sum()
}

#[tokio::test]
async fn buffered_batches_are_bounded_and_freed_without_replacing_the_channel() {
    const BATCH_BYTES: usize = 1024;
    let shared = SharedState::new(Arc::new(sample_config()));
    let publish = |batches: &mut Vec<std::sync::Weak<[Utf8Bytes]>>| {
        let frames: Arc<[Utf8Bytes]> = vec![Utf8Bytes::from("x".repeat(BATCH_BYTES))].into();
        batches.push(Arc::downgrade(&frames));
        let update = BrowserIncrementalUpdate {
            revision: 1,
            frames,
        };
        assert!(shared.browser_incremental_tx.send(update).is_ok());
    };

    let stalled = shared.subscribe_browser_incremental();
    let mut batches = Vec::new();
    for _ in 0..32 {
        publish(&mut batches);
    }
    assert_eq!(
        retained_bytes(&batches),
        BROWSER_INCREMENTAL_CHANNEL_CAPACITY * BATCH_BYTES,
        "a stalled session retains at most one channel's worth of batches"
    );
    drop(stalled);
    assert_eq!(
        retained_bytes(&batches),
        0,
        "last unsubscribe frees batches"
    );

    let mut reader = shared.subscribe_browser_incremental();
    let mut batches = Vec::new();
    publish(&mut batches);
    drop(reader.try_recv().expect("batch"));
    assert_eq!(retained_bytes(&batches), 0, "reading frees a batch");
}
