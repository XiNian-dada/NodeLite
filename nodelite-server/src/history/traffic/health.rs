//! 套餐账本的健康信号。实时用量可用不代表账本已落盘，二者需要分开观察。

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct TrafficLedgerHealth {
    /// 账本已加载且 writer 正在运行。
    pub(crate) available: bool,
    /// 最近一次落盘失败；成功后清除。
    pub(crate) write_degraded: bool,
    /// 入队被拒绝的次数。对应节点会被标记并由 writer 补写，不代表用量丢失。
    pub(crate) dropped_writes: u64,
    pub(crate) write_failures: u64,
    pub(crate) last_success_at: i64,
    /// 尚未落盘的节点数；关闭后保留最终值。
    pub(crate) pending_writes: u64,
}

impl TrafficLedgerHealth {
    pub(super) fn record_success(&mut self) {
        self.last_success_at = chrono::Utc::now().timestamp();
        self.write_degraded = false;
    }

    pub(super) fn record_failure(&mut self) {
        self.write_failures = self.write_failures.saturating_add(1);
        self.write_degraded = true;
    }
}
