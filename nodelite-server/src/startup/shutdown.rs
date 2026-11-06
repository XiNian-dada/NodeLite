//! 优雅关停回收模块：共享超时截止时间防止孤立写入任务与最终持久化竞争。

use std::future::Future;
use std::time::Duration;

use futures::StreamExt;
use futures::stream::FuturesUnordered;
use tokio::task::{JoinError, JoinHandle};
use tokio_util::sync::CancellationToken;
use tracing::warn;

pub(super) async fn await_http_shutdown(
    server: impl Future<Output = std::io::Result<()>>,
    signal: impl Future<Output = ()>,
    shutdown: CancellationToken,
    grace: Duration,
) -> std::io::Result<()> {
    tokio::pin!(server);
    tokio::select! {
        result = &mut server => {
            shutdown.cancel();
            result
        }
        _ = signal => {
            shutdown.cancel();
            match tokio::time::timeout(grace, &mut server).await {
                Ok(result) => result,
                Err(_) => {
                    warn!("HTTP shutdown deadline reached; proceeding to final persistence");
                    Ok(())
                }
            }
        }
    }
}

pub(super) async fn finish_persistence(work: impl Future<Output = ()>, grace: Duration) {
    if tokio::time::timeout(grace, work).await.is_err() {
        warn!("final persistence deadline reached; exiting shutdown");
    }
}

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

    #[tokio::test(start_paused = true)]
    async fn stalled_persistence_has_a_shared_deadline() {
        let started = Instant::now();
        super::finish_persistence(pending(), Duration::from_secs(4)).await;
        assert_eq!(started.elapsed(), Duration::from_secs(4));
    }

    struct Dropped(Arc<AtomicUsize>);

    impl Drop for Dropped {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[tokio::test(start_paused = true)]
    async fn stalled_http_does_not_prevent_final_drain() {
        let shutdown = tokio_util::sync::CancellationToken::new();
        let started = Instant::now();
        super::await_http_shutdown(
            pending(),
            async {},
            shutdown.clone(),
            Duration::from_secs(3),
        )
        .await
        .expect("deadline still permits drain");
        assert!(shutdown.is_cancelled());
        assert_eq!(started.elapsed(), Duration::from_secs(3));
        let drained = Arc::new(AtomicUsize::new(0));
        let observed = Arc::clone(&drained);
        drain_background_tasks(
            vec![tokio::spawn(async move {
                observed.fetch_add(1, Ordering::SeqCst);
            })],
            Duration::from_secs(5),
        )
        .await;
        assert_eq!(drained.load(Ordering::SeqCst), 1);
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
