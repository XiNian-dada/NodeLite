//! 网卡累计计数器基线：区分重启归零、临时回退与单调增长。

/// 判定计数器确实被重置（而非读取抖动）前的最长等待时间。
///
/// 接口被移除等情况不会让 uptime 回退，也不会恢复到原值；若无限期保持旧基线，
/// 后续用量会一直不被计入，因此回退只能短暂保持。
pub(super) const COUNTER_REGRESSION_HOLD_SECS: u64 = 300;

/// 一个 metrics 帧里与套餐计费有关的读数。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TrafficSample {
    pub(crate) rx_bytes: u64,
    pub(crate) tx_bytes: u64,
    pub(crate) uptime_secs: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct CounterBaseline {
    pub(super) rx_bytes: u64,
    pub(super) tx_bytes: u64,
    /// `None` 表示来自尚未记录 uptime 的旧账本行，此时无法确认主机没有重启。
    pub(super) uptime_secs: Option<u64>,
    regressed_since_uptime: Option<u64>,
}

impl CounterBaseline {
    pub(super) fn new(sample: TrafficSample) -> Self {
        Self::restored(sample.rx_bytes, sample.tx_bytes, Some(sample.uptime_secs))
    }

    pub(super) fn restored(rx_bytes: u64, tx_bytes: u64, uptime_secs: Option<u64>) -> Self {
        Self {
            rx_bytes,
            tx_bytes,
            uptime_secs,
            regressed_since_uptime: None,
        }
    }

    /// 推进基线并返回可计费的收发增量。回退永远不产生增量。
    pub(super) fn advance(&mut self, sample: TrafficSample) -> (u64, u64) {
        let rx_delta = sample.rx_bytes.saturating_sub(self.rx_bytes);
        let tx_delta = sample.tx_bytes.saturating_sub(self.tx_bytes);
        let regressed = sample.rx_bytes < self.rx_bytes || sample.tx_bytes < self.tx_bytes;
        let rebooted = self
            .uptime_secs
            .is_none_or(|last| sample.uptime_secs < last);
        self.rx_bytes = self.rx_bytes.max(sample.rx_bytes);
        self.tx_bytes = self.tx_bytes.max(sample.tx_bytes);
        self.uptime_secs = Some(sample.uptime_secs);

        if !regressed {
            self.regressed_since_uptime = None;
            return (rx_delta, tx_delta);
        }
        let since = *self
            .regressed_since_uptime
            .get_or_insert(sample.uptime_secs);
        if rebooted || sample.uptime_secs.saturating_sub(since) >= COUNTER_REGRESSION_HOLD_SECS {
            self.rx_bytes = sample.rx_bytes;
            self.tx_bytes = sample.tx_bytes;
            self.regressed_since_uptime = None;
        }
        (rx_delta, tx_delta)
    }
}

#[cfg(test)]
mod tests {
    use super::{COUNTER_REGRESSION_HOLD_SECS, CounterBaseline, TrafficSample};

    fn sample(rx_bytes: u64, tx_bytes: u64, uptime_secs: u64) -> TrafficSample {
        TrafficSample {
            rx_bytes,
            tx_bytes,
            uptime_secs,
        }
    }

    #[test]
    fn transient_drop_without_reboot_does_not_recount_on_recovery() {
        let mut baseline = CounterBaseline::new(sample(2_000, 1_000, 100));
        assert_eq!(baseline.advance(sample(0, 0, 110)), (0, 0));
        assert_eq!(baseline.advance(sample(2_000, 1_000, 120)), (0, 0));
        assert_eq!(baseline.advance(sample(2_010, 1_005, 130)), (10, 5));
    }

    #[test]
    fn uptime_regression_rebaselines_immediately() {
        let mut baseline = CounterBaseline::new(sample(2_000, 1_000, 100));
        assert_eq!(baseline.advance(sample(5, 3, 2)), (0, 0));
        assert_eq!(baseline.advance(sample(15, 8, 12)), (10, 5));
    }

    #[test]
    fn persistent_reset_without_reboot_rebaselines_after_the_hold_window() {
        let mut baseline = CounterBaseline::new(sample(2_000, 1_000, 100));
        assert_eq!(baseline.advance(sample(10, 10, 110)), (0, 0));
        assert_eq!(baseline.advance(sample(20, 20, 200)), (0, 0));
        let expiry = 110 + COUNTER_REGRESSION_HOLD_SECS;
        assert_eq!(baseline.advance(sample(30, 30, expiry)), (0, 0));
        assert_eq!(baseline.advance(sample(40, 45, expiry + 10)), (10, 15));
    }

    #[test]
    fn counter_wrap_never_bills_the_whole_range() {
        let mut baseline = CounterBaseline::new(sample(u64::MAX - 5, 0, 100));
        assert_eq!(baseline.advance(sample(4, 0, 110)), (0, 0));
        let expiry = 110 + COUNTER_REGRESSION_HOLD_SECS;
        assert_eq!(baseline.advance(sample(8, 0, expiry)), (0, 0));
        assert_eq!(baseline.advance(sample(18, 0, expiry + 1)), (10, 0));
    }

    #[test]
    fn one_direction_regressing_still_counts_the_other() {
        let mut baseline = CounterBaseline::new(sample(2_000, 1_000, 100));
        assert_eq!(baseline.advance(sample(0, 1_050, 110)), (0, 50));
        assert_eq!(baseline.advance(sample(2_000, 1_060, 120)), (0, 10));
    }

    #[test]
    fn recovery_restarts_the_hold_window() {
        let mut baseline = CounterBaseline::new(sample(2_000, 0, 100));
        assert_eq!(baseline.advance(sample(0, 0, 110)), (0, 0));
        assert_eq!(baseline.advance(sample(2_000, 0, 200)), (0, 0));
        assert_eq!(baseline.advance(sample(0, 0, 500)), (0, 0));
        assert_eq!(baseline.advance(sample(2_000, 0, 510)), (0, 0));
    }

    #[test]
    fn legacy_row_without_uptime_treats_a_drop_as_a_reboot() {
        let mut baseline = CounterBaseline::restored(2_000, 1_000, None);
        assert_eq!(baseline.advance(sample(5, 3, 500)), (0, 0));
        assert_eq!(baseline.advance(sample(15, 8, 510)), (10, 5));
    }
}
