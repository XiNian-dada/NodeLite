//! 网卡累计计数器基线：区分重启归零、临时回退与单调增长。

/// 判定计数器确实被重置（而非读取抖动）前的最长等待时间。
///
/// 接口被移除等情况不会让 uptime 回退，也不会恢复到原值；若无限期保持旧基线，
/// 后续用量会一直不被计入，因此回退只能短暂保持。
pub(super) const COUNTER_REGRESSION_HOLD_SECS: u64 = 300;

/// 重置后单帧增量的上限（100 Gbit/s）。超过它的跳变不可能是真实流量，只能是
/// 部分接口恢复后累计值跳回，不应计费。
const MAX_PLAUSIBLE_BYTES_PER_SEC: u64 = 12_500_000_000;

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
    /// 无重启重置前的读数。低读数持续超过保持窗口后又恢复时（如接口下线再上线），
    /// 这段字节已经计过费，需要靠它识别。必须落盘，否则重启后恢复会被重复计费。
    pub(super) rx_high_water: Option<u64>,
    pub(super) tx_high_water: Option<u64>,
    regressed_since_uptime: Option<u64>,
}

impl CounterBaseline {
    pub(super) fn new(sample: TrafficSample) -> Self {
        Self::restored(
            sample.rx_bytes,
            sample.tx_bytes,
            Some(sample.uptime_secs),
            None,
            None,
        )
    }

    pub(super) fn restored(
        rx_bytes: u64,
        tx_bytes: u64,
        uptime_secs: Option<u64>,
        rx_high_water: Option<u64>,
        tx_high_water: Option<u64>,
    ) -> Self {
        Self {
            rx_bytes,
            tx_bytes,
            uptime_secs,
            rx_high_water,
            tx_high_water,
            regressed_since_uptime: None,
        }
    }

    /// 推进基线并返回可计费的收发增量。回退本身不产生增量。
    pub(super) fn advance(&mut self, sample: TrafficSample) -> (u64, u64) {
        let regressed = sample.rx_bytes < self.rx_bytes || sample.tx_bytes < self.tx_bytes;
        let rebooted = self
            .uptime_secs
            .is_none_or(|last| sample.uptime_secs < last);
        if rebooted {
            self.rx_high_water = None;
            self.tx_high_water = None;
        }
        let interval_secs = sample
            .uptime_secs
            .saturating_sub(self.uptime_secs.unwrap_or(sample.uptime_secs))
            .max(1);
        let rx_delta = billable_delta(
            sample.rx_bytes,
            self.rx_bytes,
            &mut self.rx_high_water,
            interval_secs,
        );
        let tx_delta = billable_delta(
            sample.tx_bytes,
            self.tx_bytes,
            &mut self.tx_high_water,
            interval_secs,
        );
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
            if !rebooted {
                raise_high_water(&mut self.rx_high_water, sample.rx_bytes, self.rx_bytes);
                raise_high_water(&mut self.tx_high_water, sample.tx_bytes, self.tx_bytes);
            }
            self.rx_bytes = sample.rx_bytes;
            self.tx_bytes = sample.tx_bytes;
            self.regressed_since_uptime = None;
        }
        (rx_delta, tx_delta)
    }
}

fn billable_delta(
    current: u64,
    last: u64,
    high_water: &mut Option<u64>,
    interval_secs: u64,
) -> u64 {
    let delta = current.saturating_sub(last);
    let Some(mark) = *high_water else {
        return delta;
    };
    if current >= mark {
        *high_water = None;
        return delta.min(current - mark);
    }
    if delta > MAX_PLAUSIBLE_BYTES_PER_SEC.saturating_mul(interval_secs) {
        return 0;
    }
    delta
}

fn raise_high_water(high_water: &mut Option<u64>, current: u64, last: u64) {
    if current < last {
        *high_water = Some(high_water.map_or(last, |mark| mark.max(last)));
    }
}

#[cfg(test)]
mod tests {
    use super::{
        COUNTER_REGRESSION_HOLD_SECS, CounterBaseline, MAX_PLAUSIBLE_BYTES_PER_SEC, TrafficSample,
    };

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
        let mut baseline = CounterBaseline::restored(2_000, 1_000, None, None, None);
        assert_eq!(baseline.advance(sample(5, 3, 500)), (0, 0));
        assert_eq!(baseline.advance(sample(15, 8, 510)), (10, 5));
    }

    fn rebaselined_low(high: u64) -> (CounterBaseline, u64) {
        let mut baseline = CounterBaseline::new(sample(high, 0, 100));
        baseline.advance(sample(10, 0, 110));
        let expiry = 110 + COUNTER_REGRESSION_HOLD_SECS;
        baseline.advance(sample(20, 0, expiry));
        (baseline, expiry)
    }

    #[test]
    fn recovery_after_the_hold_window_bills_only_bytes_above_the_old_reading() {
        let (mut baseline, expiry) = rebaselined_low(2_000_000);
        assert_eq!(
            baseline.advance(sample(2_000_500, 0, expiry + 10)),
            (500, 0)
        );
        assert_eq!(baseline.rx_high_water, None);
        assert_eq!(
            baseline.advance(sample(2_000_600, 0, expiry + 20)),
            (100, 0)
        );
    }

    #[test]
    fn traffic_on_a_genuinely_reset_counter_is_still_billed() {
        let (mut baseline, expiry) = rebaselined_low(2_000_000);
        assert_eq!(baseline.advance(sample(1_020, 0, expiry + 10)), (1_000, 0));
        assert_eq!(baseline.rx_high_water, Some(2_000_000));
    }

    #[test]
    fn implausible_jump_below_the_old_reading_is_not_billed() {
        let high = MAX_PLAUSIBLE_BYTES_PER_SEC * 100;
        let (mut baseline, expiry) = rebaselined_low(high);
        let partial = MAX_PLAUSIBLE_BYTES_PER_SEC * 50;
        assert_eq!(baseline.advance(sample(partial, 0, expiry + 10)), (0, 0));
        assert_eq!(
            baseline.advance(sample(partial + 7, 0, expiry + 20)),
            (7, 0)
        );
    }

    #[test]
    fn reboot_forgets_the_old_reading() {
        let (mut baseline, expiry) = rebaselined_low(2_000_000);
        baseline.advance(sample(5, 0, expiry - 100));
        assert_eq!(baseline.rx_high_water, None);
        assert_eq!(
            baseline.advance(sample(3_000_000, 0, expiry)),
            (2_999_995, 0)
        );
    }
}
