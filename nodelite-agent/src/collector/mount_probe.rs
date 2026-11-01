//! 挂载点容量探测的超时、冷却与卡死探测记账（与平台无关，便于在任意主机上测试）。

use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, TryRecvError};
use std::time::{Duration, Instant};

use anyhow::Result;
use tracing::warn;

/// 单次探测的最长等待时间。
pub(crate) const DEFAULT_PROBE_TIMEOUT: Duration = Duration::from_secs(2);
/// 探测失败或超时后，该挂载点在此期间内直接复用上次容量而不再探测。
pub(crate) const DEFAULT_PROBE_COOLDOWN: Duration = Duration::from_secs(60);
// 探测线程只执行一次系统调用，不需要默认的 2MiB 栈。
const PROBE_THREAD_STACK_BYTES: usize = 64 * 1024;

struct MountState<T> {
    last: Option<T>,
    cooldown_until: Option<Instant>,
    // 卡在内核中的探测线程无法取消；保留其结果通道，确保同一挂载点同时最多只有一个阻塞线程。
    pending: Option<Receiver<Result<T>>>,
}

impl<T> Default for MountState<T> {
    fn default() -> Self {
        Self {
            last: None,
            cooldown_until: None,
            pending: None,
        }
    }
}

/// 按挂载点跟踪探测状态，使单个无响应挂载点只影响自身数据，且不会累积阻塞线程。
pub(crate) struct MountProbes<T> {
    mounts: HashMap<String, MountState<T>>,
    timeout: Duration,
    cooldown: Duration,
}

impl<T: Clone + Send + 'static> Default for MountProbes<T> {
    fn default() -> Self {
        Self::new(DEFAULT_PROBE_TIMEOUT, DEFAULT_PROBE_COOLDOWN)
    }
}

impl<T: Clone + Send + 'static> MountProbes<T> {
    pub(crate) fn new(timeout: Duration, cooldown: Duration) -> Self {
        Self {
            mounts: HashMap::new(),
            timeout,
            cooldown,
        }
    }

    /// 对一轮内的全部挂载点探测并按输入顺序返回容量：成功时为新值；超时、失败或冷却中时为
    /// 上次成功值（从未成功则为 `None`）。所有探测先并行启动，再共享同一个截止时间等待，
    /// 因此多个挂载点同时卡住时整轮耗时仍约为一个超时。
    pub(crate) fn probe_all<F>(
        &mut self,
        now: Instant,
        requests: Vec<(String, F)>,
    ) -> Vec<Option<T>>
    where
        F: FnOnce() -> Result<T> + Send + 'static,
    {
        let cooldown = self.cooldown;
        let mut results = Vec::with_capacity(requests.len());
        let mut waiting = Vec::new();

        for (index, (mount_point, probe)) in requests.into_iter().enumerate() {
            let state = self.mounts.entry(mount_point.clone()).or_default();
            results.push(None);
            if let Some(pending) = &state.pending {
                match pending.try_recv() {
                    Ok(result) => {
                        state.pending = None;
                        results[index] = state.apply(&mount_point, result, now, cooldown);
                        continue;
                    }
                    Err(TryRecvError::Empty) => {
                        results[index] = state.last.clone();
                        continue;
                    }
                    Err(TryRecvError::Disconnected) => state.pending = None,
                }
            }
            if state.cooldown_until.is_some_and(|until| now < until) {
                results[index] = state.last.clone();
                continue;
            }

            let (sender, receiver) = mpsc::sync_channel(1);
            let spawned = std::thread::Builder::new()
                .name("nodelite-statvfs".to_string())
                .stack_size(PROBE_THREAD_STACK_BYTES)
                .spawn(move || {
                    // 接收端可能已因挂载点消失而被丢弃，此时结果无人需要。
                    let _ = sender.send(probe());
                });
            match spawned {
                Ok(_) => waiting.push((index, mount_point, receiver)),
                Err(error) => {
                    warn!(mount_point = %mount_point, error = ?error, "failed to spawn disk probe thread");
                    state.cooldown_until = Some(now + cooldown);
                    results[index] = state.last.clone();
                }
            }
        }

        let deadline = Instant::now() + self.timeout;
        for (index, mount_point, receiver) in waiting {
            let remaining = deadline.saturating_duration_since(Instant::now());
            let state = self.mounts.entry(mount_point.clone()).or_default();
            results[index] = match receiver.recv_timeout(remaining) {
                Ok(result) => state.apply(&mount_point, result, now, cooldown),
                Err(RecvTimeoutError::Timeout) => {
                    warn!(
                        mount_point = %mount_point,
                        timeout_ms = self.timeout.as_millis() as u64,
                        "disk probe timed out; reusing last capacity until it recovers",
                    );
                    state.pending = Some(receiver);
                    state.cooldown_until = Some(now + cooldown);
                    state.last.clone()
                }
                Err(RecvTimeoutError::Disconnected) => {
                    state.cooldown_until = Some(now + cooldown);
                    state.last.clone()
                }
            };
        }
        results
    }

    #[cfg(test)]
    fn probe<F>(&mut self, mount_point: &str, now: Instant, probe: F) -> Option<T>
    where
        F: FnOnce() -> Result<T> + Send + 'static,
    {
        self.probe_all(now, vec![(mount_point.to_string(), probe)])
            .pop()
            .flatten()
    }

    /// 丢弃本轮未出现的挂载点状态，避免卸载后的条目无限增长。
    /// 仍有卡住探测的条目会保留到探测返回为止：若挂载点短暂消失后重现，
    /// 不能为同一路径再派生第二个阻塞线程。
    pub(crate) fn retain_mounts(&mut self, present: &HashSet<String>) {
        self.mounts.retain(|mount_point, state| {
            if present.contains(mount_point) {
                return true;
            }
            state
                .pending
                .as_ref()
                .is_some_and(|pending| matches!(pending.try_recv(), Err(TryRecvError::Empty)))
        });
    }

    #[cfg(test)]
    fn is_pending(&self, mount_point: &str) -> bool {
        self.mounts
            .get(mount_point)
            .is_some_and(|state| state.pending.is_some())
    }
}

impl<T: Clone> MountState<T> {
    fn apply(
        &mut self,
        mount_point: &str,
        result: Result<T>,
        now: Instant,
        cooldown: Duration,
    ) -> Option<T> {
        match result {
            Ok(value) => {
                self.cooldown_until = None;
                self.last = Some(value.clone());
                Some(value)
            }
            Err(error) => {
                warn!(
                    mount_point = %mount_point,
                    error = ?error,
                    "disk probe failed; reusing last capacity during cooldown",
                );
                self.cooldown_until = Some(now + cooldown);
                self.last.clone()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    use anyhow::anyhow;

    use super::MountProbes;

    const TIMEOUT: Duration = Duration::from_millis(20);
    const COOLDOWN: Duration = Duration::from_secs(60);

    fn probes() -> MountProbes<u64> {
        MountProbes::new(TIMEOUT, COOLDOWN)
    }

    #[test]
    fn successful_probe_returns_fresh_value() {
        let mut probes = probes();
        assert_eq!(probes.probe("/", Instant::now(), || Ok(7)), Some(7));
        assert_eq!(probes.probe("/", Instant::now(), || Ok(8)), Some(8));
    }

    #[test]
    fn failed_probe_reuses_last_value_and_cools_down() {
        let mut probes = probes();
        let start = Instant::now();
        assert_eq!(probes.probe("/data", start, || Ok(10)), Some(10));
        assert_eq!(
            probes.probe("/data", start, || Err(anyhow!("io error"))),
            Some(10)
        );

        let calls = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&calls);
        let during = probes.probe("/data", start + Duration::from_secs(1), move || {
            counted.fetch_add(1, Ordering::SeqCst);
            Ok(11)
        });
        assert_eq!(during, Some(10));
        assert_eq!(
            calls.load(Ordering::SeqCst),
            0,
            "cooldown must skip probing"
        );

        let after = probes.probe("/data", start + COOLDOWN, || Ok(12));
        assert_eq!(after, Some(12));
    }

    #[test]
    fn failed_probe_without_history_returns_none() {
        let mut probes = probes();
        assert_eq!(
            probes.probe("/mnt", Instant::now(), || Err(anyhow!("gone"))),
            None
        );
    }

    #[test]
    fn hung_probe_keeps_at_most_one_blocked_thread_per_mount() {
        let mut probes = probes();
        let start = Instant::now();
        assert_eq!(probes.probe("/nfs", start, || Ok(5)), Some(5));

        let (release, gate) = mpsc::channel::<()>();
        let spawned = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&spawned);
        let hung = probes.probe("/nfs", start, move || {
            counted.fetch_add(1, Ordering::SeqCst);
            let _ = gate.recv();
            Ok(6)
        });
        assert_eq!(hung, Some(5), "timeout reuses last capacity");
        assert!(probes.is_pending("/nfs"));

        // 冷却结束后，若旧探测仍未返回，也不能再派生新线程。
        for offset in [1, 120, 600] {
            let counted = Arc::clone(&spawned);
            let value = probes.probe("/nfs", start + Duration::from_secs(offset), move || {
                counted.fetch_add(1, Ordering::SeqCst);
                Ok(99)
            });
            assert_eq!(value, Some(5));
        }
        assert_eq!(spawned.load(Ordering::SeqCst), 1);

        release.send(()).expect("release hung probe");
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut recovered = None;
        while Instant::now() < deadline {
            let value = probes.probe("/nfs", start + Duration::from_secs(700), || Ok(7));
            if value == Some(6) {
                recovered = value;
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(
            recovered,
            Some(6),
            "late result from the hung probe is used"
        );
        assert!(!probes.is_pending("/nfs"));
    }

    #[test]
    fn hung_mount_does_not_delay_other_mounts() {
        let mut probes = probes();
        let (_release, gate) = mpsc::channel::<()>();
        let now = Instant::now();
        let started = Instant::now();
        assert_eq!(
            probes.probe("/stuck", now, move || {
                let _ = gate.recv();
                Ok(1)
            }),
            None
        );
        assert_eq!(probes.probe("/", now, || Ok(2)), Some(2));
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn several_hung_mounts_share_one_timeout() {
        let mut probes = MountProbes::<u64>::new(Duration::from_millis(200), COOLDOWN);
        let (release, gate) = mpsc::channel::<()>();
        let gate = Arc::new(std::sync::Mutex::new(gate));
        let requests = (0..5)
            .map(|i| {
                let gate = Arc::clone(&gate);
                let probe: Box<dyn FnOnce() -> anyhow::Result<u64> + Send> = Box::new(move || {
                    if let Ok(gate) = gate.lock() {
                        let _ = gate.recv();
                    }
                    Ok(1)
                });
                (format!("/stuck{i}"), probe)
            })
            .collect::<Vec<_>>();
        let started = Instant::now();
        let results = probes.probe_all(Instant::now(), requests);
        let elapsed = started.elapsed();
        assert_eq!(results, vec![None; 5]);
        assert!(
            elapsed < Duration::from_millis(600),
            "five hung mounts took {elapsed:?}, expected about one timeout"
        );
        drop(release);
    }

    #[test]
    fn retain_mounts_keeps_stuck_probe_until_it_returns() {
        let mut probes = probes();
        let start = Instant::now();
        let (release, gate) = mpsc::channel::<()>();
        let spawned = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&spawned);
        probes.probe("/nfs", start, move || {
            counted.fetch_add(1, Ordering::SeqCst);
            let _ = gate.recv();
            Ok(1)
        });
        probes.retain_mounts(&HashSet::new());
        assert!(
            probes.is_pending("/nfs"),
            "stuck probe must survive a missing mount"
        );

        let counted = Arc::clone(&spawned);
        probes.probe("/nfs", start + Duration::from_secs(600), move || {
            counted.fetch_add(1, Ordering::SeqCst);
            Ok(2)
        });
        assert_eq!(
            spawned.load(Ordering::SeqCst),
            1,
            "no second blocked thread"
        );

        release.send(()).expect("release hung probe");
        let deadline = Instant::now() + Duration::from_secs(5);
        while probes.mounts.contains_key("/nfs") && Instant::now() < deadline {
            probes.retain_mounts(&HashSet::new());
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(
            !probes.mounts.contains_key("/nfs"),
            "entry dropped once probe returned"
        );
    }

    #[test]
    fn retain_mounts_drops_unmounted_entries() {
        let mut probes = probes();
        let now = Instant::now();
        probes.probe("/a", now, || Ok(1));
        probes.probe("/b", now, || Ok(2));
        probes.retain_mounts(&HashSet::from(["/a".to_string()]));
        assert_eq!(probes.mounts.len(), 1);
        assert!(probes.mounts.contains_key("/a"));
    }
}
