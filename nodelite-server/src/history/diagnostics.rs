//! 历史数据持久化写入诊断模块：独立追踪落盘写入与队列准入，确保低负载下的写入故障可观测。

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct HistoryWriteMetrics {
    pub(crate) failures: u64,
    pub(crate) lost_samples: u64,
    pub(crate) last_success_at: i64,
    pub(crate) degraded: bool,
}

impl HistoryWriteMetrics {
    pub(super) fn record_success(&mut self) {
        self.last_success_at = chrono::Utc::now().timestamp();
        self.degraded = false;
    }

    pub(super) fn record_failure(&mut self, lost_samples: usize) {
        self.failures = self.failures.saturating_add(1);
        self.lost_samples = self.lost_samples.saturating_add(lost_samples as u64);
        self.degraded = true;
    }
}
