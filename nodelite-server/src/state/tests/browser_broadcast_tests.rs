//! 浏览器增量广播的规模边界测试。

use std::collections::HashMap;

use tokio::sync::broadcast::error::TryRecvError;

use super::super::browser_broadcast::run_tick;
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
