//! Upper bounds for alert durations, thresholds and rule ids.
//!
//! Earlier versions and the settings UI accepted larger values, so loading an existing file
//! clamps them and reports a warning instead of refusing to start. New writes through the
//! settings API use the strict check and are rejected.

use std::collections::HashSet;

use super::{AlertMetric, AlertingConfig, ConfigError, DEFAULT_HISTORY_RETENTION_HOURS};

/// 规则窗口与离线宽限上限：超过历史保留期的窗口查不到数据。
pub const MAX_ALERT_WINDOW_MINUTES: u64 = DEFAULT_HISTORY_RETENTION_HOURS * 60;
/// 巡检回看上限：历史查询本身就按保留期截断。
pub const MAX_INSPECTION_LOOKBACK_HOURS: u64 = DEFAULT_HISTORY_RETENTION_HOURS;
/// 告警冷却上限（30 天）。
pub const MAX_ALERT_COOLDOWN_MINUTES: u64 = 30 * 24 * 60;
/// 百分比类指标的阈值上限。
pub const MAX_PERCENT_THRESHOLD: u64 = 100;

/// 将超出上限的值截断到上限，返回每处调整（以及无法自动修复的重复 ID）的说明。
pub fn clamp_alerting_limits(config: &mut AlertingConfig) -> Vec<String> {
    let mut warnings = Vec::new();
    let mut seen_ids = HashSet::with_capacity(config.rules.len());
    for (index, rule) in config.rules.iter_mut().enumerate() {
        // Tracker 按规则 ID 建索引，重复 ID 会让后一条覆盖前一条；旧文件里可能已存在，
        // 只能提示，无法替运维决定保留哪一条。
        if !seen_ids.insert(rule.id.clone()) {
            warnings.push(format!(
                "alerts.rules[{index}].id duplicates an earlier rule id \"{}\"",
                rule.id
            ));
        }
        clamp(
            &mut warnings,
            &format!("alerts.rules[{index}].window_minutes"),
            &mut rule.window_minutes,
            MAX_ALERT_WINDOW_MINUTES,
        );
        clamp(
            &mut warnings,
            &format!("alerts.rules[{index}].cooldown_minutes"),
            &mut rule.cooldown_minutes,
            MAX_ALERT_COOLDOWN_MINUTES,
        );
        if is_percentage_metric(&rule.metric) {
            clamp(
                &mut warnings,
                &format!("alerts.rules[{index}].threshold"),
                &mut rule.threshold,
                MAX_PERCENT_THRESHOLD,
            );
        }
    }
    clamp(
        &mut warnings,
        "alerts.inspection.lookback_hours",
        &mut config.inspection.lookback_hours,
        MAX_INSPECTION_LOOKBACK_HOURS,
    );
    clamp(
        &mut warnings,
        "alerts.inspection.offline_grace_minutes",
        &mut config.inspection.offline_grace_minutes,
        MAX_ALERT_WINDOW_MINUTES,
    );
    warnings
}

/// 新写入的告警配置必须完全落在上限内，第一处越界即返回带字段名的错误。
pub fn check_alerting_limits(config: &AlertingConfig) -> Result<(), ConfigError> {
    let mut probe = config.clone();
    match clamp_alerting_limits(&mut probe).into_iter().next() {
        Some(message) => Err(ConfigError::new(message)),
        None => Ok(()),
    }
}

/// 流量使用率以套餐为基准，超过 100%（如 120% 超额告警）是合法用法，不在此列。
fn is_percentage_metric(metric: &AlertMetric) -> bool {
    matches!(
        metric,
        AlertMetric::CpuUsagePercent
            | AlertMetric::MemoryUsagePercent
            | AlertMetric::DiskUsagePercent
    )
}

fn clamp(warnings: &mut Vec<String>, field: &str, value: &mut u64, max: u64) {
    if *value > max {
        warnings.push(format!(
            "{field} = {} exceeds the maximum {max}; using {max}",
            *value
        ));
        *value = max;
    }
}
