//! A shared drain deadline prevents detached writers from racing final persistence.

use std::time::Duration;

use futures::StreamExt;
use futures::stream::FuturesUnordered;
use tokio::task::{JoinError, JoinHandle};
use tracing::warn;

pub(super) async fn drain_background_tasks(handles: Vec<JoinHandle<()>>, grace: Duration) {
    let mut tasks: FuturesUnordered<_> = handles.into_iter().collect();
    let drained = tokio::time::timeout(grace, async {
        while let Some(result) = tasks.next().await {
            report_join(result);
        }
    })
    .await;
    if drained.is_ok() {
        return;
    }

    warn!(
        remaining_tasks = tasks.len(),
        timeout_secs = grace.as_secs(),
        "cancelling background tasks after the shared shutdown deadline",
    );
    for task in tasks.iter() {
        task.abort();
    }
    while let Some(result) = tasks.next().await {
        report_join(result);
    }
}

fn report_join(result: Result<(), JoinError>) {
    if let Err(error) = result
        && !error.is_cancelled()
    {
        warn!(error = ?error, "background task ended with error during shutdown");
    }
}

#[cfg(test)]
mod tests {
    use std::future::pending;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use tokio::time::Instant;

    use super::drain_background_tasks;

    struct Dropped(Arc<AtomicUsize>);

    impl Drop for Dropped {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[tokio::test(start_paused = true)]
    async fn drain_stalled_tasks_with_one_deadline_and_reap_all() {
        for count in [1, 6] {
            let dropped = Arc::new(AtomicUsize::new(0));
            let tasks = (0..count)
                .map(|_| {
                    let guard = Dropped(Arc::clone(&dropped));
                    tokio::spawn(async move {
                        let _guard = guard;
                        pending::<()>().await;
                    })
                })
                .collect();
            tokio::task::yield_now().await;
            let started = Instant::now();

            drain_background_tasks(tasks, Duration::from_secs(5)).await;

            assert_eq!(started.elapsed(), Duration::from_secs(5));
            assert_eq!(dropped.load(Ordering::SeqCst), count);
        }
    }

    #[tokio::test(start_paused = true)]
    async fn drain_keeps_grace_window_for_cooperative_tasks() {
        let dropped = Arc::new(AtomicUsize::new(0));
        let tasks = (1..=3)
            .map(|seconds| {
                let guard = Dropped(Arc::clone(&dropped));
                tokio::spawn(async move {
                    let _guard = guard;
                    tokio::time::sleep(Duration::from_secs(seconds)).await;
                })
            })
            .collect();
        let started = Instant::now();

        drain_background_tasks(tasks, Duration::from_secs(5)).await;

        assert_eq!(started.elapsed(), Duration::from_secs(3));
        assert_eq!(dropped.load(Ordering::SeqCst), 3);
    }

    #[tokio::test(start_paused = true)]
    async fn drain_empty_and_failed_tasks_without_waiting_for_deadline() {
        let started = Instant::now();
        drain_background_tasks(Vec::new(), Duration::from_secs(5)).await;
        drain_background_tasks(
            vec![tokio::spawn(async { panic!("injected task failure") })],
            Duration::from_secs(5),
        )
        .await;
        assert_eq!(started.elapsed(), Duration::ZERO);
    }
}
